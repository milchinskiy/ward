use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_file(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("failed to write file");
    path
}

#[test]
fn package_path_includes_cwd_by_default() {
    // Ensure stock Lua ergonomics: with CWD on package.path, require("m") should
    // resolve ./m.lua when ward is invoked from that directory.
    let temp = tempdir().expect("tempdir");

    write_file(&temp, "m.lua", "return { ok = true }\n");
    let script = write_file(
        &temp,
        "script.lua",
        r#"local json = require("ward.convert.json")
local m = require("m")
print(json.encode(m))
"#,
    );

    let mut cmd = ward_cmd();
    cmd.current_dir(temp.path());
    cmd.args(["run", script.to_string_lossy().as_ref()]);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().expect("run output");
    assert!(
        output.status.success(),
        "lua script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    assert_eq!(value["ok"], Value::Bool(true));
}
