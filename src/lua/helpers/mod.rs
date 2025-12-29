pub mod number;
pub mod retry;
pub mod string;
pub mod table;

/// Initializes the `helpers` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    for (name, module) in [
        ("number", number::define(lua)?),
        ("retry", retry::define(lua)?),
        ("string", string::define(lua)?),
        ("table", table::define(lua)?),
    ] {
        table.set(name, module.clone())?;
        lua.register_module(format!("ward.helpers.{name}").as_str(), module)?;
    }
    Ok(table)
}
