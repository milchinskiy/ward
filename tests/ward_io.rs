use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::{TempDir, tempdir};

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script_with_input(name: &str, body: &str, input: &[u8]) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, name, body);

    let mut child = ward_cmd()
        .args(["run", script.to_string_lossy().as_ref()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn child");

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(input).expect("write stdin");
    }

    let output = child.wait_with_output().expect("wait output");

    assert!(
        output.status.success(),
        "lua script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("stdout json")
}

#[test]
fn io_read_all_reads_full_stdin_and_preserves_bytes() {
    let value = run_lua_script_with_input(
        "io_read_all.lua",
        r#"local io = require("ward.io")
local json = require("ward.convert.json")

local function to_bytes(data)
  local out = {}
  if type(data) == "string" then
    for i = 1, #data do
      out[#out + 1] = string.byte(data, i)
    end
  else
    out = data
  end
  return out
end

local function to_string(data)
  if type(data) == "string" then return data end
  local chars = {}
  for i = 1, #data do
    chars[i] = string.char(data[i])
  end
  return table.concat(chars)
end

local data = io.read_all()

print(json.encode({
  text = to_string(data),
  bytes = to_bytes(data),
}))
"#,
        b"hello\nworld!",
    );

    assert_eq!(value["text"], Value::from("hello\nworld!"));
    assert_eq!(
        value["bytes"],
        Value::from(vec![
            Value::from(104),
            Value::from(101),
            Value::from(108),
            Value::from(108),
            Value::from(111),
            Value::from(10),
            Value::from(119),
            Value::from(111),
            Value::from(114),
            Value::from(108),
            Value::from(100),
            Value::from(33),
        ])
    );
}

#[test]
fn io_read_all_enforces_max_bytes_limit() {
    let value = run_lua_script_with_input(
        "io_read_all_limit.lua",
        r#"local io = require("ward.io")
local json = require("ward.convert.json")

local ok, result = pcall(function()
  return io.read_all({ max_bytes = 3 }):wait()
end)

print(json.encode({
  ok = ok,
  err = ok and nil or tostring(result),
}))
"#,
        b"abcd",
    );

    assert_eq!(value["ok"], Value::Bool(false));
    let err = value["err"].as_str().unwrap_or_default();
    assert!(err.contains("max_bytes (3)"), "unexpected error: {err}");
}

#[test]
fn io_line_readers_strip_newlines_and_return_nil_after_eof() {
    let value = run_lua_script_with_input(
        "io_read_lines.lua",
        r#"local io = require("ward.io")
local json = require("ward.convert.json")

local function to_string(data)
  if data == nil then return nil end
  if type(data) == "string" then return data end
  local chars = {}
  for i = 1, #data do
    chars[i] = string.char(data[i])
  end
  return table.concat(chars)
end

local first = to_string(io.read_line())
local second = to_string(io.read_line())
local third = to_string(io.read_line())

local iter = io.read_lines()
local iter_first = to_string(iter())
local iter_second = to_string(iter())
local iter_third = iter()

print(json.encode({
  first = first,
  second = second,
  third = third,
  iter_first = iter_first,
  iter_second = iter_second,
  iter_third = iter_third,
}))
"#,
        b"first\nsecond\r\nthird\nfourth",
    );

    assert_eq!(value["first"], Value::from("first"));
    assert_eq!(value["second"], Value::from("second"));
    assert_eq!(value["third"], Value::from("third"));
    assert_eq!(value["iter_first"], Value::from("fourth"));
    assert!(value["iter_second"].is_null());
    assert!(value["iter_third"].is_null());
}
