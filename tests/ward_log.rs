use serde_json::Value;
use std::process::Command;
use tempfile::tempdir;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
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
fn log_module_handles_levels_and_time_via_lua() {
    let script = r#"local log = require("ward.log")
local json = require("ward.convert.json")

log.set_level("debug")
log.debug("hello", 42)

local ok_invalid, err_invalid = pcall(function()
  log.set_level("invalid")
end)

local timed = log.time("work", function()
  return { status = "done", count = 3 }
end)

print(json.encode({
  ok_invalid = ok_invalid,
  err_invalid = err_invalid and tostring(err_invalid) or nil,
  timed = timed,
}))
"#;

    let value = run_lua_script(script);

    assert_eq!(value["ok_invalid"], Value::Bool(false));
    let err = value["err_invalid"].as_str().unwrap_or_default();
    assert!(
        err.to_ascii_lowercase().contains("invalid log level"),
        "unexpected error: {err}"
    );

    assert_eq!(value["timed"]["status"], Value::from("done"));
    assert_eq!(value["timed"]["count"], Value::from(3));
}
