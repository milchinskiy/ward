use serde_json::Value;
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
fn async_spawn_reports_completion_and_results() {
    let value = run_lua_script(
        "async_spawn.lua",
        r#"local async = require("ward.async")
local time = require("ward.time")
local json = require("ward.convert.json")

local task = async.spawn(function(a, b)
  time.sleep("10ms"):wait()
  return a + b, "ok"
end, 2, 3)

local before = task:done()
local first, second = task:wait()
local after = task:done()

print(json.encode({
  before = before,
  after = after,
  results = { first, second },
}))
"#,
    );

    assert_eq!(value["before"], Value::Bool(false));
    assert_eq!(value["after"], Value::Bool(true));
    assert_eq!(value["results"][0], Value::from(5));
    assert_eq!(value["results"][1], Value::from("ok"));
}

#[test]
fn async_task_cancellation_surfaces_as_error() {
    let value = run_lua_script(
        "async_cancel.lua",
        r#"local async = require("ward.async")
local time = require("ward.time")
local json = require("ward.convert.json")

local task = async.spawn(function()
  time.sleep("200ms"):wait()
  return "late"
end)

local cancel_requested = task:cancel()
local wait_ok, wait_err = pcall(function()
  return task:wait()
end)

print(json.encode({
  cancel_requested = cancel_requested,
  wait_ok = wait_ok,
  wait_err = wait_err and tostring(wait_err) or nil,
}))
"#,
    );

    assert_eq!(value["cancel_requested"], Value::Bool(true));
    assert_eq!(value["wait_ok"], Value::Bool(false));
    let err = value["wait_err"].as_str().unwrap_or_default();
    assert!(err.contains("cancelled"), "unexpected error: {err}");
}

#[test]
fn channel_send_recv_and_close_semantics() {
    let value = run_lua_script(
        "async_channel.lua",
        r#"local async = require("ward.async")
local json = require("ward.convert.json")

local ch = async.channel({ capacity = 2 })

local send1_ok, send1_err = ch:try_send("a")
local send2_ok, send2_err = ch:try_send("b")
local send3_ok, send3_err = ch:try_send("c")

ch:close()

local recv1, recv1_err = ch:wait()
local recv2, recv2_err = ch:wait()
local recv_after_close, recv_closed_err = ch:wait()
local resend_ok, resend_err = ch:try_send("d")

print(json.encode({
  send1_ok = send1_ok,
  send1_err = send1_err,
  send2_ok = send2_ok,
  send2_err = send2_err,
  send3_ok = send3_ok,
  send3_err = send3_err,
  recv1 = recv1,
  recv1_err = recv1_err,
  recv2 = recv2,
  recv2_err = recv2_err,
  recv_after_close = recv_after_close,
  recv_closed_err = recv_closed_err,
  resend_ok = resend_ok,
  resend_err = resend_err,
}))
"#,
    );

    assert_eq!(value["send1_ok"], Value::Bool(true));
    assert!(value["send1_err"].is_null());
    assert_eq!(value["send2_ok"], Value::Bool(true));
    assert!(value["send2_err"].is_null());

    assert!(value["send3_ok"].is_null());
    assert_eq!(value["send3_err"], Value::from("full"));

    assert_eq!(value["recv1"], Value::from("a"));
    assert!(value["recv1_err"].is_null());
    assert_eq!(value["recv2"], Value::from("b"));
    assert!(value["recv2_err"].is_null());

    assert!(value["recv_after_close"].is_null());
    assert_eq!(value["recv_closed_err"], Value::from("closed"));

    assert!(value["resend_ok"].is_null());
    assert_eq!(value["resend_err"], Value::from("closed"));
}

#[test]
fn select_prefers_lowest_ready_index() {
    let value = run_lua_script(
        "async_select.lua",
        r#"local async = require("ward.async")
local time = require("ward.time")
local json = require("ward.convert.json")

local fast_idx, fast_val = async.select({ time.sleep("10ms"), time.sleep("50ms") })
local tied_idx, tied_val = async.select({ time.sleep(0), time.sleep(0) })

print(json.encode({
  fast_idx = fast_idx,
  fast_val = fast_val,
  tied_idx = tied_idx,
  tied_val = tied_val,
}))
"#,
    );

    assert_eq!(value["fast_idx"], Value::from(1));
    assert_eq!(value["fast_val"], Value::Bool(true));
    assert_eq!(value["tied_idx"], Value::from(1));
    assert_eq!(value["tied_val"], Value::Bool(true));
}
