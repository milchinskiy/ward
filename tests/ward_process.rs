use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
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
fn process_output_captures_stdout_and_status() {
    let value = run_lua_script(
        "process_output.lua",
        r#"local process = require("ward.process")
local json = require("ward.convert.json")

local result = process.cmd("sh", {"-c", "printf 'hello'"}):output()

print(json.encode({
  ok = result.ok,
  code = result.code,
  signal = result.signal,
  stdout = result.stdout,
  stderr = result.stderr,
  steps = result.steps,
}))
"#,
    );

    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["code"], Value::from(0));
    assert!(value["signal"].is_null());
    assert_eq!(value["stdout"], Value::from("hello"));
    assert!(
        value["stderr"].is_null() || value["stderr"] == "",
        "unexpected stderr: {value:?}"
    );
    assert_eq!(value["steps"][0], Value::from(0));
}

#[test]
fn pipeline_pipefail_reports_prior_stage_error() {
    let value = run_lua_script(
        "process_pipefail.lua",
        r#"local process = require("ward.process")
local json = require("ward.convert.json")

local pipeline = (process.cmd("false") | process.cmd("true")):pipefail()
local result = pipeline:output()

print(json.encode({
  ok = result.ok,
  code = result.code,
  steps = result.steps,
}))
"#,
    );

    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["code"], Value::from(0));
    assert_eq!(value["steps"][0], Value::from(1));
    assert_eq!(value["steps"][1], Value::from(0));
}
