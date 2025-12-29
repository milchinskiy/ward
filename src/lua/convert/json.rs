#![allow(clippy::needless_pass_by_value)]

use mlua::{Lua, LuaSerdeExt, Table, Value};
use serde::Serialize;

/// Initializes the `json` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let json_table = lua.create_table()?;

    // sync
    json_table.set(
        "encode",
        lua.create_function(|lua, (value, opts): (Value, Option<Table>)| encode(lua, value, opts))?,
    )?;
    json_table.set("decode", lua.create_function(|lua, input: String| decode(lua, &input))?)?;

    // async
    json_table.set(
        "encode_async",
        lua.create_async_function(|lua, (value, opts): (Value, Option<Table>)| async move {
            let (pretty, indent) = parse_options(opts)?;
            let serde_value = lua.from_value::<serde_json::Value>(value)?;
            let out = tokio::task::spawn_blocking(move || encode_json_send(serde_value, pretty, indent))
                .await
                .map_err(mlua::Error::external)?   // JoinError
                .map_err(mlua::Error::external)?;  // String

            Ok(out)
        })?,
    )?;

    json_table.set(
        "decode_async",
        lua.create_async_function(|lua, input: String| async move {
            let parsed = tokio::task::spawn_blocking(move || decode_json_send(input))
                .await
                .map_err(mlua::Error::external)?   // JoinError
                .map_err(mlua::Error::external)?;  // String

            lua.to_value(&parsed)
        })?,
    )?;

    Ok(json_table)
}

fn encode(lua: &Lua, value: Value, opts: Option<Table>) -> mlua::Result<String> {
    let (pretty, indent) = parse_options(opts)?;
    let serde_value = lua.from_value::<serde_json::Value>(value)?;

    if pretty {
        let indent_bytes = vec![b' '; indent as usize];
        let formatter = serde_json::ser::PrettyFormatter::with_indent(&indent_bytes);
        let mut serializer = serde_json::Serializer::with_formatter(Vec::new(), formatter);
        serde_value.serialize(&mut serializer).map_err(mlua::Error::external)?;
        String::from_utf8(serializer.into_inner()).map_err(mlua::Error::external)
    } else {
        serde_json::to_string(&serde_value).map_err(mlua::Error::external)
    }
}

fn decode(lua: &Lua, input: &str) -> mlua::Result<Value> {
    let value: serde_json::Value = serde_json::from_str(input).map_err(mlua::Error::external)?;
    lua.to_value(&value)
}

fn encode_json_send(value: serde_json::Value, pretty: bool, indent: u8) -> Result<String, String> {
    if indent == 0 {
        return Err("indent must be positive".to_string());
    }

    if pretty {
        let indent_bytes = vec![b' '; indent as usize];
        let formatter = serde_json::ser::PrettyFormatter::with_indent(&indent_bytes);
        let mut serializer = serde_json::Serializer::with_formatter(Vec::new(), formatter);
        value.serialize(&mut serializer).map_err(|e| e.to_string())?;
        String::from_utf8(serializer.into_inner()).map_err(|e| e.to_string())
    } else {
        serde_json::to_string(&value).map_err(|e| e.to_string())
    }
}

fn decode_json_send(input: String) -> Result<serde_json::Value, String> {
    serde_json::from_str(&input).map_err(|e| e.to_string())
}

fn parse_options(opts: Option<Table>) -> mlua::Result<(bool, u8)> {
    let Some(table) = opts else {
        return Ok((false, 2));
    };

    let pretty = table.get::<Option<bool>>("pretty")?.unwrap_or(false);
    let indent = table.get::<Option<u8>>("indent")?.unwrap_or(2);

    if indent == 0 {
        return Err(mlua::Error::external("indent must be positive"));
    }

    Ok((pretty, indent))
}
