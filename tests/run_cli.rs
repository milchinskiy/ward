use serde_json::Value;
use std::process::Command;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

#[cfg(unix)]
#[test]
fn lua_args_match_between_cli_and_shebang_invocations() {
    let temp = tempdir().expect("tempdir");
    let script_path = temp.path().join("echo_args.lua");
    let ward_path = assert_cmd::cargo::cargo_bin!("ward");

    let script_body = format!(
        "#!{} run\n{}",
        ward_path.display(),
        r#"local json = require("ward.convert.json")
local payload = { zero = _G.arg and _G.arg[0] or nil, args = {} }
if _G.arg then
  for i, v in ipairs(_G.arg) do
    table.insert(payload.args, v)
  end
end
print(json.encode(payload))
"#
    );

    std::fs::write(&script_path, script_body).expect("script write");
    let mut perms = std::fs::metadata(&script_path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).expect("chmod");

    let forwarded_args = ["foo", "bar", "baz"];

    let cli_output = ward_cmd()
        .args(["run", script_path.to_string_lossy().as_ref(), "--"])
        .args(forwarded_args)
        .output()
        .expect("cli run");
    assert!(cli_output.status.success());

    let shebang_output = Command::new(&script_path)
        .args(forwarded_args)
        .output()
        .expect("shebang run");
    assert!(shebang_output.status.success());

    let cli_json: Value = serde_json::from_slice(&cli_output.stdout).expect("cli json");
    let shebang_json: Value = serde_json::from_slice(&shebang_output.stdout).expect("shebang json");

    assert_eq!(cli_json, shebang_json, "_G.arg should match between entrypoints");
    let expected_args: Vec<Value> = forwarded_args.iter().map(|v| Value::from(*v)).collect();
    assert_eq!(cli_json["args"], Value::from(expected_args));
    assert_eq!(cli_json["zero"].as_str(), Some(script_path.to_string_lossy().as_ref()));
}

#[test]
fn lua_process_exit_code_propagates() {
    let temp = tempdir().expect("tempdir");
    let script = write_script(
        &temp,
        "exit_code.lua",
        r#"local process = require("ward.process")
process.exit(23)
"#,
    );

    let status = ward_cmd().arg("run").arg(&script).status().expect("run status");

    assert_eq!(status.code(), Some(23));
}

#[test]
fn lua_errors_produce_failure_exit_code() {
    let temp = tempdir().expect("tempdir");
    let script = write_script(
        &temp,
        "boom.lua",
        r#"error("boom")
"#,
    );

    let output = ward_cmd().arg("run").arg(&script).output().expect("run output");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn instruction_limit_is_enforced() {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, "spin.lua", "while true do end\n");

    let output = ward_cmd()
        .args(["run", "--instruction-limit", "500"])
        .arg(&script)
        .output()
        .expect("run output");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("instruction limit exceeded"));
}

#[test]
fn timeout_is_enforced() {
    let temp = tempdir().expect("tempdir");
    let script = write_script(
        &temp,
        "sleep.lua",
        r#"local time = require("ward.time")
time.sleep(0.5):wait()
"#,
    );

    let output = ward_cmd()
        .args(["run", "--timeout", "0.05"])
        .arg(&script)
        .output()
        .expect("run output");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("timed out"));
}

#[test]
fn memory_limit_is_enforced() {
    let temp = tempdir().expect("tempdir");
    let script = write_script(
        &temp,
        "memory.lua",
        r#"local s = ("x"):rep(1024 * 1024)
print(#s)
"#,
    );

    let output = ward_cmd()
        .args(["run", "--memory-limit", "32768"])
        .arg(&script)
        .output()
        .expect("run output");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.to_ascii_lowercase().contains("memory"));
}
