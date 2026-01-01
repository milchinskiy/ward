use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use tempfile::{TempDir, tempdir};

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script(name: &str, body: &str) -> (Value, ExitStatus) {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, name, body);

    let output = ward_cmd()
        .args(["run", script.to_string_lossy().as_ref()])
        .output()
        .expect("run output");

    let value = serde_json::from_slice(&output.stdout).expect("stdout json");
    (value, output.status)
}

#[test]
fn lifecycle_shutdown_handlers_run_once_in_lifo_order() {
    let (value, status) = run_lua_script(
        "lifecycle_shutdown.lua",
        r#"local lifecycle = require("ward.lifecycle")
local json = require("ward.convert.json")

local calls = {}

lifecycle.on_shutdown(function(ctx)
  calls[#calls + 1] = { name = "first", reason = ctx.reason, code = ctx.code, error = ctx.error }
end)

lifecycle.on_shutdown(function(ctx)
  calls[#calls + 1] = { name = "second", reason = ctx.reason, code = ctx.code, error = ctx.error }
end)

lifecycle.request(17)

-- Run shutdown twice; callbacks should run only once in LIFO order.
lifecycle._run_shutdown("requested", "boom")
lifecycle._run_shutdown("requested", "boom")

print(json.encode({
  calls = calls,
  requested = lifecycle.requested(),
  code = lifecycle.code(),
}))
"#,
    );

    assert_eq!(status.code(), Some(17));
    assert_eq!(value["requested"], Value::Bool(true));
    assert_eq!(value["code"], Value::from(17));

    let calls = value["calls"].as_array().expect("calls array");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["name"], Value::from("second"));
    assert_eq!(calls[1]["name"], Value::from("first"));

    for call in calls {
        assert_eq!(call["reason"], Value::from("requested"));
        assert_eq!(call["code"], Value::from(17));
        assert_eq!(call["error"], Value::from("boom"));
    }
}

#[test]
fn lifecycle_off_removes_handlers_and_tick_interrupts_when_requested() {
    let (value, status) = run_lua_script(
        "lifecycle_off.lua",
        r#"local lifecycle = require("ward.lifecycle")
local json = require("ward.convert.json")

local received = {}

local id = lifecycle.on_shutdown(function(ctx)
  received[#received + 1] = ctx.reason
end)

-- remove handler before shutdown
local removed = lifecycle.off(id)

lifecycle.request(9)

local ok, err = pcall(lifecycle._tick)
local shutdown_ok, shutdown_err = pcall(function()
  return lifecycle._run_shutdown("requested")
end)
local err_msg
if ok then
  err_msg = nil
else
  err_msg = tostring(err)
end
local shutdown_err_msg
if shutdown_ok then
  shutdown_err_msg = nil
else
  shutdown_err_msg = tostring(shutdown_err)
end
local received_len = #received

print(json.encode({
  removed = removed,
  ok = ok,
  err = err_msg,
  shutdown_ok = shutdown_ok,
  shutdown_err = shutdown_err_msg,
  received = received,
  received_len = received_len,
  requested = lifecycle.requested(),
  code = lifecycle.code(),
}))
"#,
    );

    assert_eq!(status.code(), Some(9));
    assert_eq!(value["removed"], Value::Bool(true));
    assert_eq!(value["requested"], Value::Bool(true));
    assert_eq!(value["code"], Value::from(9));

    assert_eq!(value["ok"], Value::Bool(false));
    let tick_err = value["err"].as_str().unwrap_or_default();
    assert!(tick_err.contains("interrupted"), "unexpected tick err: {tick_err}");

    assert_eq!(value["shutdown_ok"], Value::Bool(true));
    assert!(value["shutdown_err"].is_null());

    assert_eq!(value["received_len"], Value::from(0));
}
