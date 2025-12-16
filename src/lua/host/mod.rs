pub mod platform;
pub mod resources;

/// Initializes the `host` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let host = lua.create_table()?;
    host.set("platform", platform::define(lua)?)?;
    host.set("resources", resources::define(lua)?)?;
    Ok(host)
}
