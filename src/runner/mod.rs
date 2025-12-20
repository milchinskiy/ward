use mlua::{ChunkMode, HookTriggers, Lua, LuaOptions, StdLib, VmState};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::fs;

pub mod sandbox;
use sandbox::SandboxPolicy;

const TICK_EVERY: u32 = 1024;

/// Runs a lua file
/// # Errors [`crate::Error`]
pub async fn run_file(path: &Path, policy: SandboxPolicy) -> crate::Result {
    let libs = StdLib::PACKAGE | StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
    let lua_options = LuaOptions::new().thread_pool_size(policy.thread_pool_size);
    let lua = Lua::new_with(libs, lua_options).map_err(crate::Error::from)?;

    lua.set_memory_limit(policy.memory_limit_bytes)
        .map_err(crate::Error::from)?;

    // Strict instruction limiting: never exceed the configured limit.
    let remaining = Arc::new(std::sync::atomic::AtomicU64::new(policy.instruction_limit));
    let tick_counter = Arc::new(AtomicU32::new(0));
    {
        let remaining = remaining.clone();
        let tick_counter = tick_counter.clone();
        lua.set_hook(
            HookTriggers {
                #[allow(clippy::cast_possible_truncation)]
                every_nth_instruction: Some(1),
                ..HookTriggers::default()
            },
            move |lua, _debug| {
                let left_after = remaining
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| cur.checked_sub(1))
                    .map(|prev| prev - 1)
                    .unwrap_or(0);

                if left_after == 0 {
                    return Err(mlua::Error::external("script instruction limit exceeded"));
                }

                // Drain pending signals and interrupt execution if shutdown requested.
                // This must be fast and non-blocking.
                // We do it only periodically to reduce per-instruction overhead.
                let c = tick_counter.fetch_add(1, Ordering::Relaxed);
                if (c & (TICK_EVERY - 1)) == (TICK_EVERY - 1) {
                    crate::lua::lifecycle::tick(lua)?;
                }
                Ok(VmState::Continue)
            },
        )?;
    }

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

    let evaluator = lua
        .load(content)
        .set_name(name)
        .set_mode(ChunkMode::Text)
        .set_environment(env)
        .exec_async();

    // NOTE: the VM instruction hook does not execute while awaiting Rust async operations.
    // Handle Ctrl-C here so scripts can be interrupted even when blocked on I/O.
    let exec_res: crate::Result = tokio::select! {
        res = async {
            if let Some(timeout) = policy.timeout {
                match tokio::time::timeout(timeout, evaluator).await {
                    Ok(res) => res.map_err(crate::Error::from),
                    Err(e) => Err(e.into()),
                }
            } else {
                evaluator.await.map_err(crate::Error::from)
            }
        } => res,
        _ = tokio::signal::ctrl_c() => {
            // Match common shell convention: 128 + SIGINT(2) = 130.
            let _ = crate::lua::lifecycle::request_shutdown(lua, Some(130));
            Err(crate::Error::from(mlua::Error::external("interrupted")))
        }
    };

    // Decide a reason for shutdown callbacks.
    let reason = if crate::lua::lifecycle::shutdown_requested(lua) {
        crate::lua::lifecycle::ShutdownReason::Signal
    } else if matches!(exec_res, Err(crate::Error::Timeout(_))) {
        crate::lua::lifecycle::ShutdownReason::Timeout
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
