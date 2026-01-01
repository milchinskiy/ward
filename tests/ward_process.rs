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

#[test]
fn shell_defaults_and_human_timeouts_apply_to_cmds_and_pipelines() {
    let value = run_lua_script(
        "process_shell_defaults.lua",
        r#"local process = require("ward.process")
local json = require("ward.convert.json")

-- Apply only a default timeout using human-readable syntax (pipefail off).
process.shell_defaults({ pipefail = false, timeout = "100ms" })

-- Default timeout should trip on a slow command.
local slow = process.cmd("sh", {"-c", "sleep 1"}):output()

-- Enable default pipefail for pipelines.
process.shell_defaults({ pipefail = true, timeout = nil })

-- Default pipefail should mark the pipeline as not ok even though the last stage succeeds.
local pipeline = (process.cmd("sh", {"-c", "exit 1"}) | process.cmd("true")):output()

-- Turn pipefail back off and apply an explicit string timeout to a single command.
process.shell_defaults({ pipefail = false, timeout = nil })
-- Explicit string timeout should override defaults for a single command.
local short = process.cmd("sh", {"-c", "sleep 0.2"}):timeout("20ms"):output()

print(json.encode({
  slow_ok = slow.ok,
  slow_code = slow.code,
  slow_steps = slow.steps,
  pipefail_ok = pipeline.ok,
  pipefail_code = pipeline.code,
  pipefail_steps = pipeline.steps,
  short_ok = short.ok,
  short_code = short.code,
  short_steps = short.steps,
}))
"#,
    );

    assert_eq!(value["slow_ok"], Value::Bool(false));
    assert_eq!(value["slow_code"], Value::from(124));
    assert!(value["slow_steps"].as_array().is_none_or(std::vec::Vec::is_empty));

    assert_eq!(value["pipefail_ok"], Value::Bool(false));
    assert_eq!(value["pipefail_code"], Value::from(0));
    assert_eq!(value["pipefail_steps"][0], Value::from(1));
    assert_eq!(value["pipefail_steps"][1], Value::from(0));

    assert_eq!(value["short_ok"], Value::Bool(false));
    assert_eq!(value["short_code"], Value::from(124));
    assert!(value["short_steps"].as_array().is_none_or(std::vec::Vec::is_empty));
}

#[test]
fn process_middleware_wraps_commands_and_inherits_into_async_tasks() {
    let value = run_lua_script(
        "process_middleware.lua",
        r#"local process = require("ward.process")
local async = require("ward.async")
local json = require("ward.convert.json")

local function wrapper(spec)
  -- Replace argv regardless of the original program/args.
  spec.argv = {"sh", "-c", "printf 'wrapped'"}
  -- returning nil is allowed; we mutated `spec` in-place.
  return nil
end

process.push_middleware(wrapper)

local a = process.cmd("sh", {"-c", "printf 'orig'"}):output().stdout

local t = async.spawn(function()
  local r = process.cmd("sh", {"-c", "printf 'orig2'"}):output()
  return r.stdout
end)

local b = t:wait()

process.pop_middleware()

local c = process.cmd("sh", {"-c", "printf 'orig'"}):output().stdout

print(json.encode({ a = a, b = b, c = c }))
"#,
    );

    assert_eq!(value["a"], Value::from("wrapped"));
    assert_eq!(value["b"], Value::from("wrapped"));
    assert_eq!(value["c"], Value::from("orig"));
}
