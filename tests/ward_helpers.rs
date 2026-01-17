use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use tempfile::{TempDir, tempdir};

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script(name: &str, body: &str) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, name, body);

    let output = ward_cmd()
        .args(["run", script.to_string_lossy().as_ref()])
        .output()
        .expect("run output");

    assert!(
        output.status.success(),
        "lua script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("stdout json")
}

#[test]
fn helpers_string_supports_casing_trimming_and_regex() {
    let value = run_lua_script(
        "helpers_string.lua",
        r#"local helpers = require("ward.helpers")
local json = require("ward.convert.json")
local s = helpers.string

local function to_array(tbl)
  local arr = {}
  for i, v in ipairs(tbl) do
    arr[#arr + 1] = v
  end
  return arr
end

local split_values = s.split("red,green,blue", ",")
local match_one = s.match("abc123", "([a-z]+)([0-9]+)")
local match_all = s.match_all("a1 b2", "([a-z])([0-9])")
local match_replace = s.match_replace("foo123bar123", "\\d+", "X")
local match_replace_all = s.match_replace_all("2024-01-02", "\\d", "n")

local match_all_arrays = {}
for i, v in ipairs(match_all) do
  match_all_arrays[i] = to_array(v)
end

print(json.encode({
  trim = s.trim("  Hello  "),
  ltrim = s.ltrim("  Hello  "),
  rtrim = s.rtrim("  Hello  "),
  contains = s.contains("ward toolkit", "tool"),
  starts_with = s.starts_with("ward toolkit", "ward"),
  starts_with_neg = s.starts_with("ward toolkit", "tool"),
  ends_with = s.ends_with("ward toolkit", "kit"),
  ends_with_neg = s.ends_with("ward toolkit", "tool"),
  replace = s.replace("foo bar baz", "bar", "qux"),
  replace_all = s.replace_all("foo foo", "foo", "bar"),
  split = to_array(split_values),
  joined = s.join(":", table.unpack(split_values)),
  lower = s.to_lower("RustLang"),
  upper = s.to_upper("RustLang"),
  title = s.to_title("hello rust friends"),
  snake = s.to_snake("HelloRust Friends42"),
  camel = s.to_camel("hello rust friends"),
  kebab = s.to_kebab("Hello Rust Friends"),
  pascal = s.to_pascal("hello rust friends"),
  slug = s.to_slug("Hello Rust Friends"),
  match = to_array(match_one),
  match_all = match_all_arrays,
  match_replace = match_replace,
  match_replace_all = match_replace_all,
}))
"#,
    );

    assert_eq!(value["trim"], Value::from("Hello"));
    assert_eq!(value["ltrim"], Value::from("Hello  "));
    assert_eq!(value["rtrim"], Value::from("  Hello"));
    assert_eq!(value["contains"], Value::Bool(true));
    assert_eq!(value["starts_with"], Value::Bool(true));
    assert_eq!(value["starts_with_neg"], Value::Bool(false));
    assert_eq!(value["ends_with"], Value::Bool(true));
    assert_eq!(value["ends_with_neg"], Value::Bool(false));
    assert_eq!(value["replace"], Value::from("foo qux baz"));
    assert_eq!(value["replace_all"], Value::from("bar bar"));
    assert_eq!(value["split"], json!(["red", "green", "blue"]));
    assert_eq!(value["joined"], Value::from("red:green:blue"));
    assert_eq!(value["lower"], Value::from("rustlang"));
    assert_eq!(value["upper"], Value::from("RUSTLANG"));
    assert_eq!(value["title"], Value::from("Hello Rust Friends"));
    assert_eq!(value["snake"], Value::from("hello_rust_friends_42"));
    assert_eq!(value["camel"], Value::from("helloRustFriends"));
    assert_eq!(value["kebab"], Value::from("hello-rust-friends"));
    assert_eq!(value["pascal"], Value::from("HelloRustFriends"));
    assert_eq!(value["slug"], Value::from("hello-rust-friends"));
    assert_eq!(value["match"], json!(["abc123", "abc", "123"]));
    assert_eq!(value["match_all"], json!([["a1", "a", "1"], ["b2", "b", "2"]]));
    assert_eq!(value["match_replace"], Value::from("fooXbar123"));
    assert_eq!(value["match_replace_all"], Value::from("nnnn-nn-nn"));
}

#[test]
fn helpers_number_handles_predicates_and_aggregates() {
    let value = run_lua_script(
        "helpers_number.lua",
        r#"local helpers = require("ward.helpers")
local json = require("ward.convert.json")
local n = helpers.number

local nan = 0/0
local inf = math.huge

print(json.encode({
  integer = n.is_integer(5.0),
  float = n.is_float(3.25),
  number_value = n.is_number(5),
  number_string = n.is_number("5"),
  nan_check = n.is_nan(nan),
  infinity_check = n.is_infinity(inf),
  finite_check = n.is_finite(10),
  finite_nan = n.is_finite(nan),
  round = n.round(5.14159, 2),
  round_int = n.round(3.5, 0),
  clamp = n.clamp(15, 0, 10),
  sign_pos = n.sign(5.0),
  sign_neg = n.sign(-0.5),
  random_equal = n.random(7, 7),
  avg = n.avg(1, 2, 3, 4),
  min = n.min(4, 3, 2),
  max = n.max(4, 3, 5),
  sum = n.sum(1, 2, 3),
}))
"#,
    );

    assert_eq!(value["integer"], Value::Bool(true));
    assert_eq!(value["float"], Value::Bool(true));
    assert_eq!(value["number_value"], Value::Bool(true));
    assert_eq!(value["number_string"], Value::Bool(false));
    assert_eq!(value["nan_check"], Value::Bool(true));
    assert_eq!(value["infinity_check"], Value::Bool(true));
    assert_eq!(value["finite_check"], Value::Bool(true));
    assert_eq!(value["finite_nan"], Value::Bool(false));
    assert_eq!(value["round"], Value::from(5.14));
    assert_eq!(value["round_int"], Value::from(4.0));
    assert_eq!(value["clamp"], Value::from(10.0));
    assert_eq!(value["sign_pos"], Value::from(1));
    assert_eq!(value["sign_neg"], Value::from(-1));
    assert_eq!(value["random_equal"], Value::from(7.0));
    assert_eq!(value["avg"], Value::from(2.5));
    assert_eq!(value["min"], Value::from(2.0));
    assert_eq!(value["max"], Value::from(5.0));
    assert_eq!(value["sum"], Value::from(6.0));
}

#[test]
#[allow(clippy::too_many_lines)]
fn helpers_table_provides_common_mutations() {
    let value = run_lua_script(
        "helpers_table.lua",
        r#"local helpers = require("ward.helpers")
local json = require("ward.convert.json")
local t = helpers.table

local function to_array(tbl)
  local keys = {}
  for k, _ in pairs(tbl) do
    if type(k) == "number" then
      keys[#keys + 1] = k
    end
  end
  table.sort(keys)

  local arr = {}
  for _, k in ipairs(keys) do
    arr[#arr + 1] = tbl[k]
  end

  return arr
end

local function names_from(tbl)
  local names = {}
  for i, v in ipairs(tbl) do
    names[i] = v.name
  end
  return names
end

local concat_values = t.concat({ 1, 2 }, { 3 }, { 4, 5 })
local merged = t.merge({ a = 1, b = 1 }, { b = 2, c = 3 })
local deep = t.deep_merge({ config = { time = 1, nested = { first = true } }, keep = "ok" }, {
  config = { second = 2, nested = { second = true } },
})
local mapped = t.map({ 10, 20, 30 }, function(value)
  return value / 10
end)
local filtered = t.filter({ 1, 2, 3, 4 }, function(value)
  return value % 2 == 0
end)
local reduced = t.reduce({ 1, 2, 3 }, function(acc, value)
  return acc + value
end, 0)
local found = t.find({ 1, 3, 4, 6 }, function(value)
  return value % 2 == 0
end)
local found_all = t.findall({ 1, 2, 3, 4, 5, 6 }, function(value)
  return value % 2 == 0
end)
local sorted = t.sort({ 3, 1, 2 }, function(a, b)
  return a < b
end)
local reversed = t.reverse({ 9, 8, 7 })
local flattened = t.flatten({ 1, { 2, { 3 } }, 4 })
local uniqed = t.uniq({ 1, 2, 2, 3, 1 })
local uniq_by = t.uniq_by({ { name = "a" }, { name = "b" }, { name = "a" } }, function(value)
  return value.name
end)
local counted = t.count({ 1, 2, 3, 4 }, function(value)
  return value % 2 == 0
end)
local keys = t.keys({ x = 1, y = 2 })
local values = t.values({ x = 1, y = 2 })

local push_tbl = { 1, 2 }
t.push(push_tbl, 3)
local popped = t.pop(push_tbl)

local shift_tbl = { "a", "b", "c" }
local shifted = t.shift(shift_tbl)
t.prepend(shift_tbl, "z")

local joined = t.join({ "lua", "rust", 42 }, ",")

print(json.encode({
  empty_true = t.is_empty({}),
  empty_false = t.is_empty({ 1 }),
  contains = t.contains({ 1, 2, 3 }, 2),
  concat = to_array(concat_values),
  merged = merged,
  deep_time = deep.config.time,
  deep_second = deep.config.second,
  deep_nested = deep.config.nested,
  deep_keep = deep.keep,
  mapped = to_array(mapped),
  filtered = to_array(filtered),
  reduced = reduced,
  found = found,
  found_all = to_array(found_all),
  sorted = to_array(sorted),
  reversed = to_array(reversed),
  flattened = to_array(flattened),
  uniqed = to_array(uniqed),
  uniq_by = names_from(uniq_by),
  counted = counted,
  keys = to_array(keys),
  values = to_array(values),
  popped = popped,
  push_len = #push_tbl,
  shifted = shifted,
  shift_remaining = to_array(shift_tbl),
  joined = joined,
}))
"#,
    );

    assert_eq!(value["empty_true"], Value::Bool(true));
    assert_eq!(value["empty_false"], Value::Bool(false));
    assert_eq!(value["contains"], Value::Bool(true));
    assert_eq!(value["concat"], json!([1, 2, 3, 4, 5]));
    assert_eq!(value["merged"]["a"], Value::from(1));
    assert_eq!(value["merged"]["b"], Value::from(2));
    assert_eq!(value["merged"]["c"], Value::from(3));
    assert_eq!(value["deep_time"], Value::from(1));
    assert_eq!(value["deep_second"], Value::from(2));
    assert_eq!(value["deep_nested"], json!({ "first": true, "second": true }));
    assert_eq!(value["deep_keep"], Value::from("ok"));
    assert_eq!(value["mapped"], json!([1.0, 2.0, 3.0]));
    assert_eq!(value["filtered"], json!([2, 4]));
    assert_eq!(value["reduced"], Value::from(6));
    assert_eq!(value["found"], Value::from(4));
    assert_eq!(value["found_all"], json!([2, 4, 6]));
    assert_eq!(value["sorted"], json!([1, 2, 3]));
    assert_eq!(value["reversed"], json!([7, 8, 9]));
    assert_eq!(value["flattened"], json!([1, 2, 3, 4]));
    #[allow(clippy::cast_precision_loss)]
    let uniqed: Vec<f64> = value["uniqed"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .collect();
    assert_eq!(uniqed, [1.0, 2.0, 3.0]);
    assert_eq!(value["uniq_by"], json!(["a", "b"]));
    assert_eq!(value["counted"], Value::from(2));

    let mut keys: Vec<String> = value["keys"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();
    keys.sort();
    assert_eq!(keys, ["x".to_string(), "y".to_string()]);

    #[allow(clippy::cast_precision_loss)]
    let mut values: Vec<f64> = value["values"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, [1.0, 2.0]);

    assert_eq!(value["popped"], Value::from(3));
    assert_eq!(value["push_len"], Value::from(2));
    assert_eq!(value["shifted"], Value::from("a"));
    assert_eq!(value["shift_remaining"], json!(["z", "b", "c"]));
    assert_eq!(value["joined"], Value::from("lua,rust,42"));
}

#[test]
fn helpers_retry_retries_until_success() {
    let value = run_lua_script(
        "helpers_retry.lua",
        r#"local retry = require("ward.helpers.retry")
local json = require("ward.convert.json")

local attempts = 0
local result = retry.run(function()
  attempts = attempts + 1
  if attempts < 3 then
    error("try again")
  end
  return "ok"
end, { attempts = 4, delay = 0, backoff = 1.0, jitter = true, jitter_ratio = 1.0 })

local single_attempts = 0
local single = retry.run(function()
  single_attempts = single_attempts + 1
  return "done"
end, { attempts = 0, delay = 0, backoff = 0.5, max_delay = 1 })

print(json.encode({
  attempts = attempts,
  result = result,
  single_attempts = single_attempts,
  single = single,
}))
"#,
    );

    assert_eq!(value["attempts"], Value::from(3));
    assert_eq!(value["result"], Value::from("ok"));
    assert_eq!(value["single_attempts"], Value::from(1));
    assert_eq!(value["single"], Value::from("done"));
}
