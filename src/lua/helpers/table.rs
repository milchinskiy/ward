#![allow(clippy::needless_pass_by_value)]

use std::cmp::Ordering;

use mlua::{Function, Lua, Table, Value, Variadic};
use rand::rngs::SmallRng;
use rand::{SeedableRng, seq::SliceRandom};

#[allow(clippy::too_many_lines)]
/// Lua Table object
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("is_empty", lua.create_function(is_empty)?)?;
    table.set("contains", lua.create_function(contains)?)?;
    table.set("concat", lua.create_function(concat)?)?;
    table.set("merge", lua.create_function(merge)?)?;
    table.set("deep_merge", lua.create_function(deep_merge)?)?;
    table.set("map", lua.create_function(map)?)?;
    table.set("filter", lua.create_function(filter)?)?;
    table.set("reduce", lua.create_function(reduce)?)?;
    table.set("each", lua.create_function(each)?)?;
    table.set("find", lua.create_function(find)?)?;
    table.set("findall", lua.create_function(find_all)?)?;
    table.set("sort", lua.create_function(sort)?)?;
    table.set("reverse", lua.create_function(reverse)?)?;
    table.set("shuffle", lua.create_function(shuffle)?)?;
    table.set("flatten", lua.create_function(flatten)?)?;
    table.set("uniq", lua.create_function(uniq)?)?;
    table.set("uniq_by", lua.create_function(uniq_by)?)?;
    table.set("count", lua.create_function(count)?)?;
    table.set("keys", lua.create_function(keys)?)?;
    table.set("values", lua.create_function(values)?)?;
    table.set("push", lua.create_function(push)?)?;
    table.set("append", lua.create_function(append)?)?;
    table.set("pop", lua.create_function(pop)?)?;
    table.set("shift", lua.create_function(shift)?)?;
    table.set("prepend", lua.create_function(prepend)?)?;
    table.set("join", lua.create_function(join)?)?;

    Ok(table)
}

fn is_empty(_: &Lua, tbl: Table) -> mlua::Result<bool> {
    let mut iter = tbl.pairs::<Value, Value>();
    Ok(iter.next().transpose()?.is_none())
}

fn contains(_: &Lua, (tbl, value): (Table, Value)) -> mlua::Result<bool> {
    for result in tbl.pairs::<Value, Value>() {
        let (_, current) = result?;
        if current == value {
            return Ok(true);
        }
    }
    Ok(false)
}

fn concat(lua: &Lua, (first, rest): (Table, Variadic<Table>)) -> mlua::Result<Table> {
    let mut values = collect_sequence_values(&first)?;
    for tbl in rest {
        values.extend(collect_sequence_values(&tbl)?);
    }

    table_from_values(lua, &values)
}

fn merge(lua: &Lua, (first, rest): (Table, Variadic<Table>)) -> mlua::Result<Table> {
    let merged = lua.create_table()?;
    copy_table(&first, &merged)?;
    for tbl in rest {
        copy_table(&tbl, &merged)?;
    }
    Ok(merged)
}

fn deep_merge(lua: &Lua, (first, rest): (Table, Variadic<Table>)) -> mlua::Result<Table> {
    let base = deep_copy_table(lua, &first)?;
    let merged = rest
        .into_iter()
        .try_fold(base, |current, tbl| deep_merge_tables(lua, &current, &tbl))?;
    Ok(merged)
}

fn map(lua: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Table> {
    let mapped = lua.create_table()?;
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let new_value = func.call::<Value>((value, key.clone()))?;
        mapped.set(key, new_value)?;
    }
    Ok(mapped)
}

fn filter(lua: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Table> {
    let filtered = lua.create_table()?;
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if func.call::<bool>((value.clone(), key.clone()))? {
            filtered.set(key, value)?;
        }
    }
    Ok(filtered)
}

fn reduce(_: &Lua, (tbl, func, mut acc): (Table, Function, Value)) -> mlua::Result<Value> {
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        acc = func.call((acc, value, key))?;
    }
    Ok(acc)
}

fn each(_: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Table> {
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        func.call::<()>((value, key))?;
    }
    Ok(tbl)
}

fn find(_: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Value> {
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if func.call::<bool>((value.clone(), key))? {
            return Ok(value);
        }
    }
    Ok(Value::Nil)
}

fn find_all(lua: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Table> {
    let mut matches = Vec::new();
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if func.call::<bool>((value.clone(), key))? {
            matches.push(value);
        }
    }
    table_from_values(lua, &matches)
}

fn sort(lua: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Table> {
    let mut values = collect_sequence_values(&tbl)?;
    let mut error = None;

    values.sort_by(|a, b| match func.call::<bool>((a.clone(), b.clone())) {
        Ok(less) => {
            if less {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        Err(e) => {
            error = Some(e);
            Ordering::Equal
        }
    });

    if let Some(err) = error {
        return Err(err);
    }

    table_from_values(lua, &values)
}

fn reverse(lua: &Lua, tbl: Table) -> mlua::Result<Table> {
    let mut values = collect_sequence_values(&tbl)?;
    values.reverse();
    table_from_values(lua, &values)
}

fn shuffle(lua: &Lua, tbl: Table) -> mlua::Result<Table> {
    let mut values = collect_sequence_values(&tbl)?;
    let mut rng = SmallRng::from_os_rng();
    values.shuffle(&mut rng);
    table_from_values(lua, &values)
}

fn flatten(lua: &Lua, tbl: Table) -> mlua::Result<Table> {
    let mut values = Vec::new();
    flatten_values(&tbl, &mut values)?;
    table_from_values(lua, &values)
}

fn uniq(lua: &Lua, tbl: Table) -> mlua::Result<Table> {
    let mut seen = Vec::new();
    let mut unique = Vec::new();

    for value in collect_sequence_values(&tbl)? {
        if !seen.iter().any(|v| v == &value) {
            seen.push(value.clone());
            unique.push(value);
        }
    }

    table_from_values(lua, &unique)
}

fn uniq_by(lua: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<Table> {
    let mut seen_keys = Vec::new();
    let mut unique = Vec::new();

    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let uniq_key_value = func.call::<Value>((value.clone(), key))?;
        if !seen_keys.iter().any(|existing| existing == &uniq_key_value) {
            seen_keys.push(uniq_key_value);
            unique.push(value);
        }
    }

    table_from_values(lua, &unique)
}

fn count(_: &Lua, (tbl, func): (Table, Function)) -> mlua::Result<i64> {
    let mut count = 0;
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if func.call::<bool>((value, key))? {
            count += 1;
        }
    }
    Ok(count)
}

fn keys(lua: &Lua, tbl: Table) -> mlua::Result<Table> {
    let mut keys = Vec::new();
    for pair in tbl.pairs::<Value, Value>() {
        let (key, _) = pair?;
        keys.push(key);
    }
    table_from_values(lua, &keys)
}

fn values(lua: &Lua, tbl: Table) -> mlua::Result<Table> {
    let mut values = Vec::new();
    for pair in tbl.pairs::<Value, Value>() {
        let (_, value) = pair?;
        values.push(value);
    }
    table_from_values(lua, &values)
}

fn push(_: &Lua, (tbl, value): (Table, Value)) -> mlua::Result<()> {
    let len = tbl.raw_len();
    #[allow(clippy::cast_possible_wrap)]
    tbl.set((len + 1) as i64, value)?;
    Ok(())
}

fn append(lua: &Lua, args: (Table, Value)) -> mlua::Result<()> {
    push(lua, args)
}

fn pop(_: &Lua, tbl: Table) -> mlua::Result<Value> {
    let len = tbl.raw_len();
    if len == 0 {
        return Ok(Value::Nil);
    }

    #[allow(clippy::cast_possible_wrap)]
    let last_index = len as i64;
    let value = tbl.get::<Value>(last_index)?;
    tbl.set(last_index, Value::Nil)?;
    Ok(value)
}

fn shift(_: &Lua, tbl: Table) -> mlua::Result<Value> {
    let len = tbl.raw_len();
    if len == 0 {
        return Ok(Value::Nil);
    }

    let first_value = tbl.get::<Value>(1)?;
    for idx in 1..len {
        #[allow(clippy::cast_possible_wrap)]
        let next_idx = (idx + 1) as i64;
        let value = tbl.get::<Value>(next_idx)?;
        #[allow(clippy::cast_possible_wrap)]
        tbl.set(idx as i64, value)?;
    }
    #[allow(clippy::cast_possible_wrap)]
    tbl.set(len as i64, Value::Nil)?;
    Ok(first_value)
}

fn prepend(_: &Lua, (tbl, value): (Table, Value)) -> mlua::Result<()> {
    let len = tbl.raw_len();
    for idx in (1..=len).rev() {
        #[allow(clippy::cast_possible_wrap)]
        let target = (idx + 1) as i64;
        #[allow(clippy::cast_possible_wrap)]
        let current = tbl.get::<Value>(idx as i64)?;
        tbl.set(target, current)?;
    }
    tbl.set(1, value)?;
    Ok(())
}

fn join(_: &Lua, (tbl, sep): (Table, String)) -> mlua::Result<String> {
    let mut buffer = String::new();
    let len = tbl.raw_len();
    for idx in 1..=len {
        if idx > 1 {
            buffer.push_str(&sep);
        }
        #[allow(clippy::cast_possible_wrap)]
        let value: Value = tbl.get(idx as i64)?;
        buffer.push_str(&value_to_string(&value));
    }
    Ok(buffer)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.to_string_lossy(),
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Table(_) => "[table]".to_string(),
        Value::Function(_) => "[function]".to_string(),
        Value::Thread(_) => "[thread]".to_string(),
        Value::UserData(_) => "[userdata]".to_string(),
        Value::LightUserData(_) => "[lightuserdata]".to_string(),
        Value::Error(err) => format!("[error {err}]"),
        Value::Other(_) => "[other]".to_string(),
    }
}

fn collect_sequence_values(tbl: &Table) -> mlua::Result<Vec<Value>> {
    let mut values = Vec::with_capacity(tbl.raw_len().max(1));
    for value in tbl.sequence_values::<Value>() {
        values.push(value?);
    }
    Ok(values)
}

#[allow(clippy::cast_possible_wrap)]
fn table_from_values(lua: &Lua, values: &[Value]) -> mlua::Result<Table> {
    let table = lua.create_table_with_capacity(values.len(), 0)?;
    for (idx, value) in values.iter().enumerate() {
        table.set((idx + 1) as i64, value.clone())?;
    }
    Ok(table)
}

fn flatten_values(tbl: &Table, acc: &mut Vec<Value>) -> mlua::Result<()> {
    for value in tbl.sequence_values::<Value>() {
        let value = value?;
        match value {
            Value::Table(inner) => flatten_values(&inner, acc)?,
            other => acc.push(other),
        }
    }
    Ok(())
}

fn copy_table(from: &Table, to: &Table) -> mlua::Result<()> {
    for pair in from.pairs::<Value, Value>() {
        let (key, value) = pair?;
        to.set(key, value)?;
    }
    Ok(())
}

fn deep_copy_table(lua: &Lua, tbl: &Table) -> mlua::Result<Table> {
    let copy = lua.create_table_with_capacity(tbl.raw_len(), 0)?;
    for pair in tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        copy.set(key, deep_copy_value(lua, &value)?)?;
    }
    Ok(copy)
}

fn deep_copy_value(lua: &Lua, value: &Value) -> mlua::Result<Value> {
    match value {
        Value::Table(tbl) => Ok(Value::Table(deep_copy_table(lua, tbl)?)),
        other => Ok(other.clone()),
    }
}

fn deep_merge_tables(lua: &Lua, base: &Table, next: &Table) -> mlua::Result<Table> {
    let merged = deep_copy_table(lua, base)?;
    merge_into(lua, &merged, next)?;
    Ok(merged)
}

fn merge_into(lua: &Lua, destination: &Table, source: &Table) -> mlua::Result<()> {
    for pair in source.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if let Value::Table(src_table) = value {
            if let Value::Table(dest_table) = destination.get::<Value>(key.clone())? {
                let merged = deep_merge_tables(lua, &dest_table, &src_table)?;
                destination.set(key, merged)?;
            } else {
                destination.set(key, deep_copy_table(lua, &src_table)?)?;
            }
        } else {
            destination.set(key, value)?;
        }
    }
    Ok(())
}
