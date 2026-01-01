pub mod unix;

/// Initializes the `ipc` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let ipc = lua.create_table()?;

    let unix = unix::define(lua)?;
    ipc.set("unix", unix.clone())?;
    lua.register_module("ward.ipc.unix", unix)?;

    Ok(ipc)
}
