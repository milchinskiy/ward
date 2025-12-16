use mlua::{ChunkMode, HookTriggers, Lua, LuaOptions, StdLib, VmState};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};
use tokio::fs;

pub mod sandbox;
use sandbox::SandboxPolicy;

/// Runs a lua file
/// # Errors [`crate::Error`]
pub async fn run_file(path: &Path, policy: SandboxPolicy) -> crate::Result {
    let libs = StdLib::PACKAGE | StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
    let lua_options = LuaOptions::new().thread_pool_size(policy.thread_pool_size);
    let lua = Lua::new_with(libs, lua_options).map_err(crate::Error::from)?;

    lua.set_memory_limit(policy.memory_limit_bytes)
        .map_err(crate::Error::from)?;

    let remaining = Arc::new(std::sync::atomic::AtomicU64::new(policy.instruction_limit));
    {
        let remaining = remaining.clone();
        lua.set_hook(
            HookTriggers {
                every_nth_instruction: Some(10_000),
                ..HookTriggers::default()
            },
            move |lua, _debug| {
                let step = 10_000u64;
                let left = remaining
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| cur.checked_sub(step))
                    .unwrap_or(0);

                if left == 0 {
                    return Err(mlua::Error::external("script instruction limit exceeded"));
                }
                // Drain pending signals and interrupt execution if shutdown requested.
                // This must be fast and non-blocking.
                crate::lua::lifecycle::tick(lua)?;
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

    // Run the chunk, but always run shutdown callbacks before returning.
    let exec_res: crate::Result = if let Some(timeout) = policy.timeout {
        match tokio::time::timeout(timeout, evaluator).await {
            Ok(res) => res.map_err(crate::Error::from),
            Err(e) => Err(e.into()),
        }
    } else {
        evaluator.await.map_err(crate::Error::from)
    };

    // Decide a reason for shutdown callbacks.
    let reason = if crate::lua::lifecycle::shutdown_requested(lua) {
        crate::lua::lifecycle::ShutdownReason::Signal
    } else if exec_res.is_err() {
        crate::lua::lifecycle::ShutdownReason::Error
    } else {
        crate::lua::lifecycle::ShutdownReason::Success
    };

    let shut_res = crate::lua::lifecycle::run_shutdown(lua, reason, None);
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
        lua.register_module(format!("ward.{name}").as_str(), &module)?;
        exposed_modules.set(name, module)?;
    }
    lua.register_module("ward", exposed_modules)?;

    Ok(())
}
