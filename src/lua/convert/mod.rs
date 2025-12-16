pub mod json;
pub mod toml;
pub mod yaml;
pub mod ini;

/// Initializes the `convert` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    for (name, module) in [
        ("json", json::define(lua)?),
        ("toml", toml::define(lua)?),
        ("yaml", yaml::define(lua)?),
        ("ini", ini::define(lua)?),
    ] {
        table.set(name, &module)?;
        lua.register_module(format!("ward.convert.{name}").as_str(), module)?;
    }
    Ok(table)
}
