use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use tempfile::{tempdir, TempDir};

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
fn json_encode_and_decode_supports_pretty_and_async() {
    let value = run_lua_script(
        "convert_json.lua",
        r#"local json = require("ward.convert.json")

local payload = { foo = "bar", numbers = { 1, 2, 3 }, nested = { answer = 42 } }
local encoded = json.encode(payload)
local decoded = json.decode(encoded)

local pretty = json.encode(payload, { pretty = true, indent = 4 })
local async_encoded = json.encode_async(payload)
local async_decoded = json.decode_async(encoded)

local ok_indent, indent_err = pcall(function()
  return json.encode(payload, { indent = 0 })
end)

print(json.encode({
  encoded = encoded,
  pretty = pretty,
  async_encoded = async_encoded,
  decoded = decoded,
  async_decoded = async_decoded,
  indent_ok = ok_indent,
  indent_err = indent_err and tostring(indent_err) or nil,
}))
"#,
    );

    let encoded = value["encoded"].as_str().expect("encoded json");
    assert_eq!(encoded, value["async_encoded"].as_str().expect("async json"));
    assert_eq!(value["decoded"]["foo"], Value::from("bar"));
    assert_eq!(value["decoded"]["numbers"], json!([1, 2, 3]));
    assert_eq!(value["decoded"]["nested"]["answer"], Value::from(42));
    assert_eq!(value["async_decoded"]["numbers"], json!([1, 2, 3]));
    let pretty = value["pretty"].as_str().expect("pretty string");
    assert!(pretty.contains('\n'), "pretty output should contain newlines");
    assert!(
        pretty.contains("    \"foo\""),
        "expected pretty output to include indentation"
    );
    assert_eq!(value["indent_ok"], Value::Bool(false));
    let err = value["indent_err"].as_str().unwrap_or_default();
    assert!(
        err.contains("indent must be positive"),
        "unexpected indent error: {err}"
    );
}

#[test]
fn toml_and_yaml_round_trip_with_async_variants() {
    let value = run_lua_script(
        "convert_toml_yaml.lua",
        r#"local json = require("ward.convert.json")
local toml = require("ward.convert.toml")
local yaml = require("ward.convert.yaml")

local payload = { title = "Ward", numbers = { prime = 13 }, list = { "a", "b" } }

local toml_encoded = toml.encode(payload)
local toml_decoded = toml.decode(toml_encoded)
local toml_async_decoded = toml.decode_async(toml_encoded)

local yaml_encoded = yaml.encode(payload)
local yaml_decoded = yaml.decode(yaml_encoded)
local yaml_async_encoded = yaml.encode_async(payload)

print(json.encode({
  toml = {
    encoded = toml_encoded,
    decoded = toml_decoded,
    async_decoded = toml_async_decoded,
  },
  yaml = {
    encoded = yaml_encoded,
    decoded = yaml_decoded,
    async_encoded = yaml_async_encoded,
  },
}))
"#,
    );

    assert_eq!(value["toml"]["decoded"]["title"], Value::from("Ward"));
    assert_eq!(value["toml"]["decoded"]["numbers"]["prime"], Value::from(13));
    assert_eq!(value["toml"]["async_decoded"]["list"][0], Value::from("a"));

    assert!(value["yaml"]["encoded"].as_str().unwrap_or_default().contains("title: Ward"));
    assert_eq!(value["yaml"]["decoded"]["numbers"]["prime"], Value::from(13));
    assert_eq!(value["yaml"]["async_encoded"], value["yaml"]["encoded"]);
}

#[test]
fn ini_encode_decode_and_error_propagation() {
    let value = run_lua_script(
        "convert_ini.lua",
        r#"local json = require("ward.convert.json")
local ini = require("ward.convert.ini")

local payload = {
  [""] = { root = "ok" },
  server = { host = "localhost", port = 8080 },
}

local encoded = ini.encode(payload)
local decoded = ini.decode(encoded)
local async_encoded = ini.encode_async(payload)
local async_decoded = ini.decode_async(encoded)

local invalid_ok, invalid_err = pcall(function()
  return ini.encode({ bad = { count = { 1, 2, 3 } } })
end)

print(json.encode({
  encoded = encoded,
  async_encoded = async_encoded,
  decoded = decoded,
  async_decoded = async_decoded,
  invalid_ok = invalid_ok,
  invalid_err = invalid_err and tostring(invalid_err) or nil,
}))
"#,
    );

    let encoded = value["encoded"].as_str().expect("encoded ini");
    assert!(encoded.contains("host=localhost"));
    assert!(encoded.contains("port=8080"));
    let async_encoded = value["async_encoded"].as_str().expect("async ini");
    assert!(async_encoded.contains("host=localhost"));
    assert!(async_encoded.contains("port=8080"));
    assert_eq!(value["decoded"]["server"]["host"], Value::from("localhost"));
    assert_eq!(value["decoded"]["server"]["port"], Value::from("8080"));
    assert_eq!(value["async_decoded"][""]["root"], Value::from("ok"));
    assert_eq!(value["async_decoded"]["server"]["port"], Value::from("8080"));

    assert_eq!(value["invalid_ok"], Value::Bool(false));
    let err = value["invalid_err"].as_str().unwrap_or_default();
    assert!(
        err.contains("ini values must be boolean, number, or string"),
        "unexpected ini encode error: {err}"
    );
}
