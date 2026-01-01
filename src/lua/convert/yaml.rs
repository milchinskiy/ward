#![allow(clippy::needless_pass_by_value)]

use mlua::{Lua, LuaSerdeExt, Table, Value};

/// Initializes the `yaml` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let yaml_table = lua.create_table()?;

    // sync
    yaml_table.set("encode", lua.create_function(|lua, value: Value| encode(lua, value))?)?;
    yaml_table.set("decode", lua.create_function(|lua, input: String| decode(lua, &input))?)?;

    // async
    yaml_table.set(
        "encode_async",
        lua.create_async_function(|lua, value: Value| async move {
            let serde_value = lua.from_value::<serde_yaml::Value>(value)?;
            let out = tokio::task::spawn_blocking(move || yaml_encode_send(serde_value))
                .await
                .map_err(mlua::Error::external)? // JoinError
                .map_err(mlua::Error::external)?; // String

            Ok(out)
        })?,
    )?;

    yaml_table.set(
        "decode_async",
        lua.create_async_function(|lua, input: String| async move {
            let parsed = tokio::task::spawn_blocking(move || yaml_decode_send(input))
                .await
                .map_err(mlua::Error::external)? // JoinError
                .map_err(mlua::Error::external)?; // String
            lua.to_value(&parsed)
        })?,
    )?;

    Ok(yaml_table)
}

fn encode(lua: &Lua, value: Value) -> mlua::Result<String> {
    let serde_value = lua.from_value::<serde_yaml::Value>(value)?;
    serde_yaml::to_string(&serde_value).map_err(mlua::Error::external)
}

fn decode(lua: &Lua, input: &str) -> mlua::Result<Value> {
    let value: serde_yaml::Value = serde_yaml::from_str(input).map_err(mlua::Error::external)?;
    lua.to_value(&value)
}

fn yaml_encode_send(value: serde_yaml::Value) -> Result<String, String> {
    serde_yaml::to_string(&value).map_err(|e| e.to_string())
}

fn yaml_decode_send(input: String) -> Result<serde_yaml::Value, String> {
    serde_yaml::from_str(&input).map_err(|e| e.to_string())
}
