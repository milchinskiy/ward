pub mod http;
pub mod fetch;

/// Initializes the `net` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let net = lua.create_table()?;
    net.set("http", http::define(lua)?)?;
    net.set("fetch", fetch::define(lua)?)?;
    Ok(net)
}
