use mlua::{ChunkMode, HookTriggers, Lua, LuaOptions, StdLib, Table, VmState};
use std::{
    ffi::OsString,
    path::Path,
    sync::{Arc, atomic::Ordering},
};
use tokio::fs;

pub mod sandbox;
use sandbox::SandboxPolicy;

const HOOK_STRIDE: u32 = 1024;

/// Runs a lua file
/// # Errors [`crate::Error`]
pub async fn run_file(path: &Path, args: &[OsString], policy: SandboxPolicy) -> crate::Result {
    let libs = StdLib::PACKAGE | StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
    let lua_options = LuaOptions::new().thread_pool_size(policy.thread_pool_size);
    let lua = Lua::new_with(libs, lua_options)?;

    lua.set_memory_limit(policy.memory_limit_bytes)?;
    // NOTE: the script is executed inside a Lua coroutine when using `exec_async()`,
    // therefore instruction hooks must be installed on the coroutine that executes the chunk.
    // Hook installation happens in `evaluate()`.

    let mut lua_content = fs::read_to_string(path).await?;
    // drop shebang if present
    if lua_content.starts_with("#!") {
        if let Some(pos) = lua_content.find('\n') {
            lua_content.replace_range(..=pos, "\n");
        } else {
            // clear entire file if shebang is the only line
            lua_content.clear();
        }
    }

    let name = path.to_string_lossy().to_string();
    evaluate(&lua, &lua_content, name.as_str(), args, &policy).await
}

async fn evaluate(lua: &Lua, content: &str, name: &str, args: &[OsString], policy: &SandboxPolicy) -> crate::Result {
    lua.set_app_data(policy.clone());
    populate_modules(lua, policy)?;

    let env = lua.globals();
    populate_env(lua, &env, name, args)?;

    // Approximate instruction limiting: the VM hook runs every HOOK_STRIDE instructions (or less),
    // so the script may exceed the configured limit by up to (hook_stride - 1) instructions.
    // This trades strictness for significantly lower overhead.
    //
    // WARN: Lua hooks are per-thread. `exec_async()` executes the chunk inside a coroutine,
    // so the hook must be installed on that coroutine (not only on the main Lua thread).
    let hook_stride: u32 = if policy.instruction_limit == u64::MAX {
        HOOK_STRIDE
    } else {
        // If a small limit is configured, keep the hook at or below that limit.
        let s = policy.instruction_limit.min(u64::from(HOOK_STRIDE));
        if s == 0 {
            1
        } else {
            u32::try_from(s).unwrap_or(u32::MAX)
        }
    };

    let remaining: Option<Arc<std::sync::atomic::AtomicU64>> = if policy.instruction_limit == u64::MAX {
        None
    } else {
        Some(Arc::new(std::sync::atomic::AtomicU64::new(policy.instruction_limit)))
    };

    let func = lua
        .load(content)
        .set_name(name)
        .set_mode(ChunkMode::Text)
        .set_environment(env)
        .into_function()?;

    let thread = lua.create_thread(func)?;

    {
        let remaining = remaining.clone();
        let step = u64::from(hook_stride);
        thread.set_hook(
            HookTriggers {
                every_nth_instruction: Some(hook_stride),
                ..HookTriggers::default()
            },
            move |lua, _debug| {
                if let Some(ref remaining) = remaining {
                    let prev = remaining.fetch_sub(step, Ordering::Relaxed);
                    if prev <= step {
                        // Prevent wrap-around on underflow (defensive; we are about to error anyway).
                        remaining.store(0, Ordering::Relaxed);
                        return Err(mlua::Error::external("instruction limit exceeded"));
                    }
                }

                // Drain pending signals and interrupt execution if shutdown requested.
                // The hook itself is already coarse, so do this on every hook call.
                crate::lua::lifecycle::tick(lua)?;
                Ok(VmState::Continue)
            },
        )?;
    }

    let evaluator = crate::lua::process::proc_middleware_scope(lua.clone(), Vec::new(), thread.into_async::<()>(())?);

    // NOTE: the VM instruction hook does not execute while awaiting Rust async operations.
    // Handle Ctrl-C here so scripts can be interrupted even when blocked on I/O.
    let exec_res: crate::Result = tokio::select! {
        res = async {
            if let Some(timeout) = policy.timeout {
                match tokio::time::timeout(timeout, evaluator).await {
                    Ok(res) => res.map_err(Into::into),
                    Err(e) => Err(e.into()),
                }
            } else {
                evaluator.await.map_err(Into::into)
            }
        } => res,
        _ = tokio::signal::ctrl_c() => {
            // Match common shell convention: 128 + SIGINT(2) = 130.
            let _ = crate::lua::lifecycle::request_shutdown_signal(lua, Some(130));
            Err(crate::Error::from(mlua::Error::external("interrupted")))
        }
    };

    let exec_was_exit_requested = matches!(exec_res, Err(crate::Error::Lua(ref e))
        if crate::lua::lifecycle::is_exit_requested_error(e));

    // Decide a reason for shutdown callbacks.
    let reason = if matches!(exec_res, Err(crate::Error::Timeout(_))) {
        crate::lua::lifecycle::ShutdownReason::Timeout
    } else if crate::lua::lifecycle::shutdown_requested(lua) {
        crate::lua::lifecycle::shutdown_origin(lua).unwrap_or(crate::lua::lifecycle::ShutdownReason::Requested)
    } else if exec_res.is_err() {
        crate::lua::lifecycle::ShutdownReason::Error
    } else {
        crate::lua::lifecycle::ShutdownReason::Success
    };

    let error = exec_res.as_ref().err().map(std::string::ToString::to_string);
    let shut_res = crate::lua::lifecycle::run_shutdown(lua, reason, error);
    let mut out: crate::Result = match (exec_res, shut_res) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(e)) => Err(e.into()),
        (Err(e), _) => Err(e),
    };

    // If the VM asked for shutdown with an explicit exit code, prefer that over generic errors.
    // IMPORTANT: code=0 must not mask real errors. We only treat code=0 as success when execution
    // was interrupted by `process.exit(0)` (ExitRequested marker).
    if crate::lua::lifecycle::shutdown_requested(lua)
        && let Some(code) = crate::lua::lifecycle::shutdown_code(lua)
    {
        if code != 0 {
            out = Err(crate::Error::Exit(code));
        } else if exec_was_exit_requested {
            out = Ok(());
        }
    }

    out
}

#[allow(clippy::missing_const_for_fn)]
fn populate_env(lua: &Lua, env: &mlua::Table, script_name: &str, args: &[OsString]) -> mlua::Result<()> {
    let arg_table = lua.create_table_with_capacity(args.len(), 1)?;
    arg_table.set(0, script_name)?;
    for (idx, arg) in args.iter().enumerate() {
        arg_table.set(idx + 1, arg.to_string_lossy().into_owned())?;
    }
    env.set("arg", arg_table)?;
    Ok(())
}

#[allow(unused_variables)]
fn populate_modules(lua: &Lua, policy: &SandboxPolicy) -> mlua::Result<()> {
    let exposed_modules = lua.create_table()?;
    let existing_modules = crate::lua::modules(lua)?;

    for (name, module) in existing_modules {
        lua.register_module(format!("ward.{name}").as_str(), module.clone())?;
        exposed_modules.set(name, module)?;
    }
    lua.register_module("ward", exposed_modules)?;

    // Lua ergonomics: `require("foo")` should search the current working directory
    // by default, as stock Lua does (./?.lua;./?/init.lua).
    ensure_cwd_in_package_path(lua)?;

    Ok(())
}

fn ensure_cwd_in_package_path(lua: &Lua) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let path: String = package.get("path")?;
    let parts: Vec<&str> = path.split(';').collect();

    // NOTE: prevent system-installed Lua C modules from being loaded
    // as they may not be compatible with the ward runtime/sandbox
    package.set("cpath", "")?;
    package.set(
        "loadlib",
        mlua::Value::Function(lua.create_function(|_, (_libname, _funcname): (mlua::Value, mlua::Value)| {
            Ok((mlua::Value::Nil, "C modules are disabled by Ward".to_string()))
        })?),
    )?;

    // Match Lua's default search order.
    let mut prefix: Vec<&str> = Vec::new();
    if !parts.contains(&"./?.lua") {
        prefix.push("./?.lua");
    }
    if !parts.contains(&"./?/init.lua") {
        prefix.push("./?/init.lua");
    }

    if prefix.is_empty() {
        return Ok(());
    }

    let mut new_path = String::new();
    for p in prefix {
        new_path.push_str(p);
        new_path.push(';');
    }
    new_path.push_str(&path);
    package.set("path", new_path)?;
    Ok(())
}
