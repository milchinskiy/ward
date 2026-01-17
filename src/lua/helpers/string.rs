#![allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]

use mlua::{Lua, Table, Variadic};
use regex::Regex;
use std::sync::OnceLock;

#[allow(clippy::too_many_lines)]
/// Lua String object
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let string = lua.create_table()?;
    string.set("trim", lua.create_function(trim)?)?;
    string.set("ltrim", lua.create_function(ltrim)?)?;
    string.set("rtrim", lua.create_function(rtrim)?)?;
    string.set("contains", lua.create_function(contains)?)?;
    string.set("starts_with", lua.create_function(starts_with)?)?;
    string.set("ends_with", lua.create_function(ends_with)?)?;
    string.set("replace", lua.create_function(replace)?)?;
    string.set("replace_all", lua.create_function(replace_all)?)?;
    string.set("split", lua.create_function(split)?)?;
    string.set("join", lua.create_function(join)?)?;
    string.set("to_lower", lua.create_function(to_lower)?)?;
    string.set("to_upper", lua.create_function(to_upper)?)?;
    string.set("to_title", lua.create_function(to_title)?)?;
    string.set("to_snake", lua.create_function(to_snake)?)?;
    string.set("to_camel", lua.create_function(to_camel)?)?;
    string.set("to_kebab", lua.create_function(to_kebab)?)?;
    string.set("to_pascal", lua.create_function(to_pascal)?)?;
    string.set("to_slug", lua.create_function(to_slug)?)?;
    string.set("match", lua.create_function(regex_match)?)?;
    string.set("match_all", lua.create_function(match_all)?)?;
    string.set("match_replace", lua.create_function(match_replace)?)?;
    string.set("match_replace_all", lua.create_function(match_replace_all)?)?;

    Ok(string)
}

fn trim(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(s.trim().to_string())
}

fn ltrim(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(s.trim_start().to_string())
}

fn rtrim(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(s.trim_end().to_string())
}

fn contains(_: &Lua, (s, substr): (String, String)) -> mlua::Result<bool> {
    Ok(s.contains(&substr))
}

fn starts_with(_: &Lua, (s, substr): (String, String)) -> mlua::Result<bool> {
    Ok(s.starts_with(&substr))
}

fn ends_with(_: &Lua, (s, substr): (String, String)) -> mlua::Result<bool> {
    Ok(s.ends_with(&substr))
}

fn replace(_: &Lua, (s, substr, replacement): (String, String, String)) -> mlua::Result<String> {
    Ok(s.replacen(&substr, &replacement, 1))
}

fn replace_all(_: &Lua, (s, substr, replacement): (String, String, String)) -> mlua::Result<String> {
    Ok(s.replace(&substr, &replacement))
}

fn split(lua: &Lua, (s, sep): (String, String)) -> mlua::Result<Table> {
    let parts = s.split(&sep).map(ToString::to_string);
    sequence_from_strings(lua, parts)
}

fn join(_: &Lua, (sep, parts): (String, Variadic<String>)) -> mlua::Result<String> {
    let mut result = String::new();
    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            result.push_str(&sep);
        }
        result.push_str(part);
    }
    Ok(result)
}

fn to_lower(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(s.to_lowercase())
}

fn to_upper(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(s.to_uppercase())
}

fn to_title(_: &Lua, s: String) -> mlua::Result<String> {
    let words = split_words(&s);
    let mut result = String::new();
    for (idx, word) in words.iter().enumerate() {
        if idx > 0 {
            result.push(' ');
        }
        result.push_str(&capitalize(word));
    }
    Ok(result)
}

fn to_snake(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(join_words(&s, '_'))
}

fn to_camel(_: &Lua, s: String) -> mlua::Result<String> {
    let words = split_words(&s);
    let mut result = String::new();
    for (idx, word) in words.iter().enumerate() {
        if idx == 0 {
            result.push_str(&word.to_lowercase());
        } else {
            result.push_str(&capitalize(word));
        }
    }
    Ok(result)
}

fn to_kebab(_: &Lua, s: String) -> mlua::Result<String> {
    Ok(join_words(&s, '-'))
}

fn to_pascal(_: &Lua, s: String) -> mlua::Result<String> {
    let words = split_words(&s);
    let mut result = String::new();
    for word in &words {
        result.push_str(&capitalize(word));
    }
    Ok(result)
}

fn to_slug(lua: &Lua, s: String) -> mlua::Result<String> {
    to_kebab(lua, s)
}

fn regex_match(lua: &Lua, (s, pattern): (String, String)) -> mlua::Result<Table> {
    let regex = build_regex(&pattern)?;
    regex.captures(&s).map_or_else(
        || sequence_from_strings(lua, std::iter::empty()),
        |caps| capture_to_table(lua, caps.iter()),
    )
}

fn match_all(lua: &Lua, (s, pattern): (String, String)) -> mlua::Result<Table> {
    let regex = build_regex(&pattern)?;
    let matches = regex.captures_iter(&s);
    let outer = lua.create_table()?;

    for (idx, caps) in matches.enumerate() {
        let table = capture_to_table(lua, caps.iter())?;
        #[allow(clippy::cast_possible_wrap)]
        outer.set((idx + 1) as i64, table)?;
    }

    Ok(outer)
}

fn match_replace(_: &Lua, (s, pattern, replacement): (String, String, String)) -> mlua::Result<String> {
    let regex = build_regex(&pattern)?;
    let replaced = regex.replace(&s, replacement.as_str());
    Ok(replaced.into_owned())
}

fn match_replace_all(_: &Lua, (s, pattern, replacement): (String, String, String)) -> mlua::Result<String> {
    let regex = build_regex(&pattern)?;
    let replaced = regex.replace_all(&s, replacement.as_str());
    Ok(replaced.into_owned())
}

fn build_regex(pattern: &str) -> mlua::Result<Regex> {
    Regex::new(pattern).map_err(|err| mlua::Error::RuntimeError(format!("Invalid pattern: {err}")))
}

fn split_words(s: &str) -> Vec<String> {
    word_regex().find_iter(s).map(|m| m.as_str().to_string()).collect()
}

fn join_words(s: &str, separator: char) -> String {
    let words = split_words(s);
    let mut result = String::new();
    for (idx, word) in words.iter().enumerate() {
        if idx > 0 {
            result.push(separator);
        }
        result.push_str(&word.to_lowercase());
    }
    result
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut result = String::with_capacity(word.len());
    result.extend(first.to_uppercase());
    result.push_str(chars.as_str().to_lowercase().as_str());
    result
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn sequence_from_strings(lua: &Lua, values: impl IntoIterator<Item = String>) -> mlua::Result<Table> {
    let collected: Vec<String> = values.into_iter().collect();
    let table = lua.create_table_with_capacity(collected.len(), 0)?;
    for (idx, value) in collected.iter().enumerate() {
        table.set((idx + 1) as i64, value.clone())?;
    }
    Ok(table)
}

#[allow(clippy::cast_possible_wrap)]
fn capture_to_table<'a>(lua: &'a Lua, captures: impl Iterator<Item = Option<regex::Match<'a>>>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (idx, m) in captures.enumerate() {
        if let Some(v) = m {
            table.set((idx + 1) as i64, v.as_str().to_string())?;
        }
    }
    Ok(table)
}

fn word_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"[A-Z]+([A-Z][a-z]|[0-9]|$)|[A-Z]?[a-z]+|[0-9]+").expect("word regex must be valid")
    })
}
