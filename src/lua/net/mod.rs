pub mod http;
pub mod fetch;

/// Initializes the `net` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let net = lua.create_table()?;
    for (name, module) in [
        ("http", http::define(lua)?),
        ("fetch", fetch::define(lua)?),
    ] {
        net.set(name, &module)?;
        lua.register_module(format!("ward.net.{name}").as_str(), module)?;
    }
    Ok(net)
}
