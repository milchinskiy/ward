pub mod number;
pub mod string;
pub mod table;

/// Initializes the `type` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    table.set("number", number::define(lua)?)?;
    table.set("string", string::define(lua)?)?;
    table.set("table", table::define(lua)?)?;
    Ok(table)
}
