#![cfg(unix)]

use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use tempfile::{TempDir, tempdir};

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    fs::write(&path, body).expect("failed to write lua script");
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
fn unix_ipc_roundtrip() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("roundtrip.sock");

    let script = format!(
        r#"local async = require("ward.async")
local unix = require("ward.ipc.unix")
local json = require("ward.convert.json")

local path = "{path}"

local listener = assert(unix.listen(path))
local accepted = async.spawn(function()
  local stream, err = listener:accept()
  assert(stream, err)
  local data, read_err = stream:read(5)
  assert(data, read_err)
  local ok, write_err = stream:write_all("pong")
  assert(ok, write_err)
  return data
end)

local client = assert(unix.connect(path))
local ok, werr = client:write_all("ping")
assert(ok, werr)
local response, rerr = client:read(4)
assert(response, rerr)

listener:close()

local accepted_data = accepted:wait()
print(json.encode({{ recv = accepted_data, response = response }}))
"#,
        path = socket_path.display(),
    );

    let value = run_lua_script("ipc_roundtrip.lua", &script);
    assert_eq!(value["recv"], Value::from("ping"));
    assert_eq!(value["response"], Value::from("pong"));
}

#[test]
fn unix_ipc_removes_stale_socket() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("stale.sock");
    fs::write(&socket_path, b"stale").expect("write stale marker");

    let script = format!(
        r#"local unix = require("ward.ipc.unix")
local json = require("ward.convert.json")

local listener = assert(unix.listen("{path}"))
listener:close()

print(json.encode({{ ok = true }}))
"#,
        path = socket_path.display(),
    );

    let value = run_lua_script("ipc_stale.lua", &script);
    assert_eq!(value["ok"], Value::Bool(true));
    assert!(!socket_path.exists(), "socket file should be removed");
}

#[test]
fn unix_ipc_sets_mode() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("mode.sock");

    let script = format!(
        r#"local unix = require("ward.ipc.unix")
local json = require("ward.convert.json")

local listener = assert(unix.listen("{path}", {{
  mode = tonumber("660", 8),
  unlink_on_close = false,
  mkdir = true,
}}))

print(json.encode({{ ok = listener ~= nil }}))
"#,
        path = socket_path.display(),
    );

    let value = run_lua_script("ipc_mode.lua", &script);
    assert_eq!(value["ok"], Value::Bool(true));

    let metadata = fs::metadata(&socket_path).expect("metadata");
    let mode = metadata.permissions().mode() & 0o7777;
    assert_eq!(mode, 0o660);

    fs::remove_file(&socket_path).expect("cleanup socket");
}

#[test]
fn unix_ipc_unlinks_on_close_by_default() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("unlink.sock");

    let script = format!(
        r#"local unix = require("ward.ipc.unix")
local json = require("ward.convert.json")

local listener = assert(unix.listen("{path}"))
listener:close()

print(json.encode({{ ok = true }}))
"#,
        path = socket_path.display(),
    );

    let value = run_lua_script("ipc_unlink.lua", &script);
    assert_eq!(value["ok"], Value::Bool(true));
    assert!(!socket_path.exists(), "socket file should be unlinked");
}
