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
    let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
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
            move |_lua, _debug| {
                let step = 10_000u64;
                let left = remaining
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| cur.checked_sub(step))
                    .unwrap_or(0);

                if left == 0 {
                    return Err(mlua::Error::external("script instruction limit exceeded"));
                }
                Ok(VmState::Continue)
            },
        )?;
    }

    let mut lua_content = fs::read_to_string(path).await?;
    // drop shebang if present
    if lua_content.starts_with("#!") {
        lua_content = lua_content.lines().skip(1).collect::<Vec<_>>().join("\n");
    }

    let name = path.to_string_lossy().to_string();
    evaluate(&lua, &lua_content, name.as_str(), &policy).await
}

async fn evaluate(lua: &Lua, content: &str, name: &str, policy: &SandboxPolicy) -> crate::Result {
    use crate::runner::sandbox::SandboxPolicyPermissions;
    lua.set_app_data(SandboxPolicyPermissions::from(policy));
    populate_modules(lua, policy)?;

    let env = lua.create_table()?;
    populate_env(lua, &env, policy)?;

    let evaluator = lua
        .load(content)
        .set_name(name)
        .set_mode(ChunkMode::Text)
        .set_environment(env)
        .exec_async();

    if let Some(timeout) = policy.timeout {
        tokio::time::timeout(timeout, evaluator).await??;
    } else {
        evaluator.await?;
    }

    Ok(())
}

fn populate_env(lua: &Lua, env: &mlua::Table, policy: &SandboxPolicy) -> mlua::Result<()> {
    let globals = lua.globals();
    let safe = lua.create_table()?;

    for (name, allowed) in &policy.globals {
        let value = if *allowed {
            globals.get::<mlua::Value>(name.as_str())?
        } else {
            mlua::Value::Nil
        };
        safe.set(name.as_str(), value)?;
    }

    let mt = lua.create_table()?;
    mt.set("__index", safe)?;
    env.set_metatable(Some(mt))?;

    if policy.allow_require {
        // TODO: implement require
    } else {
        env.set("require", mlua::Value::Nil)?;
    }

    // Avoid giving access to raw globals table
    env.set("_G", env.clone())?;

    Ok(())
}

fn populate_modules(lua: &Lua, policy: &SandboxPolicy) -> mlua::Result<()> {
    let exposed_modules = lua.create_table()?;
    let allowed_modules = crate::lua::modules(lua)?
        .into_iter()
        .filter(|(name, _)| policy.ward_modules.contains_key(name) && policy.ward_modules[name])
        .collect::<Vec<_>>();

    for (name, module) in allowed_modules {
        lua.register_module(format!("ward.{name}").as_str(), &module)?;
        exposed_modules.set((*name).to_string(), module)?;
    }
    lua.register_module("ward", exposed_modules)?;

    Ok(())
}
