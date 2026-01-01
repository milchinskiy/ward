#![allow(clippy::needless_pass_by_value)]

use mlua::{Lua, LuaSerdeExt, Table, Value};

/// Initializes the `toml` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let toml_table = lua.create_table()?;

    // sync
    toml_table.set("encode", lua.create_function(|lua, value: Value| encode(lua, value))?)?;
    toml_table.set("decode", lua.create_function(|lua, input: String| decode(lua, &input))?)?;

    // async
    toml_table.set(
        "encode_async",
        lua.create_async_function(|lua, value: Value| async move {
            let serde_value = lua.from_value::<toml::Value>(value)?;
            let out = tokio::task::spawn_blocking(move || toml_encode_send(serde_value))
                .await
                .map_err(mlua::Error::external)? // JoinError
                .map_err(mlua::Error::external)?; // String -> ExternalError

            Ok(out)
        })?,
    )?;

    toml_table.set(
        "decode_async",
        lua.create_async_function(|lua, input: String| async move {
            let parsed = tokio::task::spawn_blocking(move || toml_decode_send(input))
                .await
                .map_err(mlua::Error::external)? // JoinError
                .map_err(mlua::Error::external)?; // String
            lua.to_value(&parsed)
        })?,
    )?;

    Ok(toml_table)
}

fn encode(lua: &Lua, value: Value) -> mlua::Result<String> {
    let serde_value = lua.from_value::<toml::Value>(value)?;
    toml::to_string(&serde_value).map_err(mlua::Error::external)
}

fn decode(lua: &Lua, input: &str) -> mlua::Result<Value> {
    let value: toml::Value = toml::from_str(input).map_err(mlua::Error::external)?;
    lua.to_value(&value)
}

fn toml_encode_send(value: toml::Value) -> Result<String, String> {
    toml::to_string(&value).map_err(|e| e.to_string())
}

fn toml_decode_send(input: String) -> Result<toml::Value, String> {
    toml::from_str(&input).map_err(|e| e.to_string())
}
