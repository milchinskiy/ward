pub mod platform;
pub mod resources;

/// Initializes the `host` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let host = lua.create_table()?;
    for (name, module) in [
        ("platform", platform::define(lua)?),
        ("resources", resources::define(lua)?),
    ] {
        host.set(name, module.clone())?;
        lua.register_module(format!("ward.host.{name}").as_str(), module)?;
    }
    Ok(host)
}
