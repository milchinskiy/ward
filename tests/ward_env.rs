use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script(body: &str) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, "script.lua", body);

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
fn env_overlay_and_path_resolution_are_applied_in_lua() {
    let temp = tempdir().expect("tempdir");
    let binary_name = if cfg!(windows) {
        "ward_env_integration.exe"
    } else {
        "ward_env_integration"
    };
    let binary_path = temp.path().join(binary_name);
    std::fs::write(&binary_path, "#!/bin/sh\necho ok").expect("binary write");
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&binary_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary_path, perms).expect("chmod");
    }

    let path_literal = serde_json::to_string(&temp.path().to_string_lossy()).expect("path literal");
    let binary_literal = serde_json::to_string(binary_name).expect("binary literal");
    let pathext_literal = serde_json::to_string(if cfg!(windows) { ".EXE" } else { "" }).unwrap();

    let script = format!(
        r#"local env = require("ward.env")
local json = require("ward.convert.json")

env.set("WARD_TEST_KEY", "value")
local got = env.get("WARD_TEST_KEY", "missing")
local exists = env.is_exists("WARD_TEST_KEY")

env.unset("WARD_TEST_KEY")
local fallback = env.get("WARD_TEST_KEY", "fallback")
local exists_after_unset = env.is_exists("WARD_TEST_KEY")

env.set("WARD_SECOND", "yes")
env.set("PATH", {path_literal})
local pathext = {pathext_literal}
if pathext ~= "" then
  env.set("PATHEXT", pathext)
end

local listed = env.list()
local which = env.which({binary_literal})
local in_path = env.is_in_path({binary_literal})

env.clear()
local listed_after_clear = env.list()

print(json.encode({{
  got = got,
  exists = exists,
  fallback = fallback,
  exists_after_unset = exists_after_unset,
  listed_second = listed["WARD_SECOND"],
  which = which,
  in_path = in_path,
  cleared_has_key = listed_after_clear["WARD_SECOND"] ~= nil,
}}))
"#,
    );

    let value = run_lua_script(&script);

    assert_eq!(value["got"], Value::from("value"));
    assert_eq!(value["exists"], Value::Bool(true));
    assert_eq!(value["fallback"], Value::from("fallback"));
    assert_eq!(value["exists_after_unset"], Value::Bool(false));
    assert_eq!(value["listed_second"], Value::from("yes"));
    assert_eq!(value["which"], Value::from(binary_path.to_string_lossy().as_ref()));
    assert_eq!(value["in_path"], Value::Bool(true));
    assert_eq!(value["cleared_has_key"], Value::Bool(false));
}
