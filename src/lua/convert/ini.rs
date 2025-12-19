#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unnecessary_wraps)]

use configparser::ini::Ini;
use mlua::{Lua, Table, Value};

use std::collections::BTreeMap;

type IniDoc = BTreeMap<String, BTreeMap<String, String>>;

/// Initializes the `ini` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let ini_table = lua.create_table()?;

    // sync
    ini_table.set("encode", lua.create_function(|_, value: Value| encode(value))?)?;
    ini_table.set("decode", lua.create_function(|lua, input: String| decode(lua, &input))?)?;

    // async
    ini_table.set(
        "encode_async",
        lua.create_async_function(|_, value: Value| async move {
            let doc = value_to_ini_doc(value)?;
            let out = tokio::task::spawn_blocking(move || encode_ini_send(doc))
                .await
                .map_err(mlua::Error::external)?   // JoinError
                .map_err(mlua::Error::external)?;  // String

            Ok(out)
        })?,
    )?;

    ini_table.set(
        "decode_async",
        lua.create_async_function(|lua, input: String| async move {
            let doc = tokio::task::spawn_blocking(move || decode_ini(input.as_str()))
                .await
                .map_err(mlua::Error::external)?   // JoinError
                .map_err(mlua::Error::external)?;  // String

            ini_doc_to_lua(&lua, doc)
        })?,
    )?;

    Ok(ini_table)
}

fn encode(value: Value) -> mlua::Result<String> {
    let doc = value_to_ini_doc(value)?;
    Ok(encode_ini(doc))
}

fn decode(lua: &Lua, input: &str) -> mlua::Result<Value> {
    let doc = decode_ini(input).map_err(mlua::Error::external)?;
    ini_doc_to_lua(lua, doc)
}

fn value_to_ini_doc(value: Value) -> mlua::Result<IniDoc> {
    let Value::Table(table) = value else {
        return Err(mlua::Error::external("ini.encode expects a table"));
    };

    let mut doc: IniDoc = BTreeMap::new();

    for pair in table.pairs::<String, Value>() {
        let (section_name, section_value) = pair?;
        let Value::Table(section_table) = section_value else {
            return Err(mlua::Error::external("ini section must be a table"));
        };

        let mut props = BTreeMap::new();
        for entry in section_table.pairs::<String, Value>() {
            let (key, raw_value) = entry?;
            let rendered = value_to_string(raw_value)?;
            props.insert(key, rendered);
        }

        doc.insert(section_name, props);
    }

    Ok(doc)
}

fn ini_doc_to_lua(lua: &Lua, doc: IniDoc) -> mlua::Result<Value> {
    let result = lua.create_table()?;
    for (section, props) in doc {
        let section_table = lua.create_table()?;
        for (k, v) in props {
            section_table.set(k, v)?;
        }
        result.set(section, section_table)?;
    }
    Ok(Value::Table(result))
}

// Send-safe worker for spawn_blocking
fn encode_ini_send(doc: IniDoc) -> Result<String, String> {
    Ok(encode_ini(doc))
}

fn encode_ini(doc: IniDoc) -> String {
    let mut ini = Ini::new();
    ini.set_default_section("");

    for (section_name, props) in doc {
        if section_name.is_empty() {
            ini.set_default_section("");
        }
        let section_key = if section_name.is_empty() { "" } else { section_name.as_str() };

        for (k, v) in props {
            ini.set(section_key, &k, Some(v));
        }
    }

    ini.writes()
}

fn decode_ini(input: &str) -> Result<IniDoc, String> {
    let mut ini = Ini::new();
    ini.set_default_section("");

    let map = ini.read(input.to_owned())?;
    let mut doc: IniDoc = BTreeMap::new();

    for (section, props) in map {
        let mut section_map = BTreeMap::new();
        for (k, v) in props {
            section_map.insert(k, v.unwrap_or_default());
        }
        doc.insert(section, section_map);
    }

    Ok(doc)
}

fn value_to_string(value: Value) -> mlua::Result<String> {
    match value {
        Value::Boolean(v) => Ok(v.to_string()),
        Value::Integer(v) => Ok(v.to_string()),
        Value::Number(v) => Ok(v.to_string()),
        Value::String(s) => s.to_str().map(|s| s.to_string()).map_err(mlua::Error::external),
        other => Err(mlua::Error::external(format!(
            "ini values must be boolean, number, or string, got {other:?}"
        ))),
    }
}

