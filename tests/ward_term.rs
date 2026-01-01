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
fn term_progress_and_ansi_helpers_work_without_tty() {
    let script = r#"local term = require("ward.term")
local json = require("ward.convert.json")

local ansi = term.ansi
local color = ansi.red .. "hi" .. ansi.reset
local tty_stdout = term.isatty("stdout")
local tty_stderr = term.isatty("stderr")

local prog = term.progress({ total = 3, message = "Start", width = 10, stream = "stderr" })
local initial_total = prog:total()
prog:tick()
local after_first = prog:value()
prog:value(2)
local after_set = prog:value()
prog:message("Updated")
local message_before_finish = prog:message()
local finish_ok = prog:finish("Done")
local message_after_finish = prog:message()
local finished_total = prog:total()

print(json.encode({
  color = color,
  tty_stdout = tty_stdout,
  tty_stderr = tty_stderr,
  initial_total = initial_total,
  after_first = after_first,
  after_set = after_set,
  message_before_finish = message_before_finish,
  message_after_finish = message_after_finish,
  finish_ok = finish_ok,
  finished_total = finished_total,
}))
"#;

    let value = run_lua_script(script);

    assert_eq!(value["color"], Value::from("\u{1b}[31mhi\u{1b}[0m"));
    assert_eq!(value["tty_stdout"], Value::Bool(false));
    assert_eq!(value["tty_stderr"], Value::Bool(false));
    assert_eq!(value["initial_total"], Value::from(3));
    assert_eq!(value["after_first"], Value::from(1));
    assert_eq!(value["after_set"], Value::from(2));
    assert_eq!(value["message_before_finish"], Value::from("Updated"));
    assert_eq!(value["message_after_finish"], Value::from("Done"));
    assert_eq!(value["finish_ok"], Value::Bool(true));
    assert_eq!(value["finished_total"], Value::from(3));
}
