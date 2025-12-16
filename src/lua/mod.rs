pub mod convert;
pub mod env;
pub mod fs;
pub mod helpers;
pub mod http;
pub mod io;

#[allow(unused)]
fn require(
    lua: &mlua::Lua,
    f: impl FnOnce(&crate::runner::sandbox::SandboxPolicy) -> bool,
    msg: &'static str,
) -> mlua::Result<()> {
    lua.app_data_ref::<crate::runner::sandbox::SandboxPolicy>()
        .and_then(|p| f(&p).then_some(()))
        .ok_or_else(|| mlua::Error::external(msg))
}

/// Returns a list of modules and their definitions
/// # Errors [`crate::Error`]
pub fn modules(lua: &mlua::Lua) -> mlua::Result<Vec<(String, mlua::Table)>> {
    Ok(vec![
        ("convert".to_string(), convert::define(lua)?),
        ("env".to_string(), env::define(lua)?),
        ("fs".to_string(), fs::define(lua)?),
        ("io".to_string(), io::define(lua)?),
        ("helpers".to_string(), helpers::define(lua)?),
        ("http".to_string(), http::define(lua)?),
    ])
}
