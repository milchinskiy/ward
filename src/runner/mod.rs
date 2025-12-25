use mlua::{ChunkMode, HookTriggers, Lua, LuaOptions, StdLib, VmState};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::Ordering,
    },
};
use tokio::fs;

pub mod sandbox;
use sandbox::SandboxPolicy;

const HOOK_STRIDE: u32 = 1024;

/// Runs a lua file
/// # Errors [`crate::Error`]
pub async fn run_file(path: &Path, policy: SandboxPolicy) -> crate::Result {
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
    evaluate(&lua, &lua_content, name.as_str(), &policy).await
}

async fn evaluate(lua: &Lua, content: &str, name: &str, policy: &SandboxPolicy) -> crate::Result {
    lua.set_app_data(policy.clone());
    populate_modules(lua, policy)?;

    let env = lua.globals();
    populate_env(lua, &env, policy)?;

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
        if s == 0 { 1 } else { u32::try_from(s).unwrap_or(u32::MAX) }
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

    let evaluator = thread.into_async::<()>(())?;

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
    match (exec_res, shut_res) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(e)) => Err(e.into()),
        (Err(e), _) => Err(e),
    }
}

#[allow(unused_variables, clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn populate_env(lua: &Lua, env: &mlua::Table, policy: &SandboxPolicy) -> mlua::Result<()> {
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

    // Enable `require("externals.<name>")` by installing a dedicated searcher.
    crate::lua::module::install_externals_searcher(lua)?;

    Ok(())
}
