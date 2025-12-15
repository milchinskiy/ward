pub mod json;
pub mod toml;
pub mod yaml;
pub mod ini;

/// Initializes the `convert` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    table.set("json", json::define(lua)?)?;
    table.set("toml", toml::define(lua)?)?;
    table.set("yaml", yaml::define(lua)?)?;
    table.set("ini", ini::define(lua)?)?;
    Ok(table)
}
