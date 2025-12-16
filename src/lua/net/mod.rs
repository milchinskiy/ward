pub mod http;

/// Initializes the `net` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let net = lua.create_table()?;
    net.set("http", http::define(lua)?)?;
    Ok(net)
}
