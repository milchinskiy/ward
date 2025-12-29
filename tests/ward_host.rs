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
fn host_platform_and_resources_match_runtime_expectations() {
    let script = r#"local platform = require("ward.host.platform")
local resources = require("ward.host.resources")
local json = require("ward.convert.json")

local flags = {
  windows = platform.is_windows(),
  macos = platform.is_macos(),
  linux = platform.is_linux(),
  unix = platform.is_unix(),
  bsd = platform.is_bsd(),
}

local info = platform.info()
local shell = platform.shell()
local res = resources.get()

print(json.encode({
  flags = flags,
  info = info,
  shell = shell,
  resources = {
    memory = res.memory,
    cpu = {
      logical = res.cpu.cores.logical,
      physical = res.cpu.cores.physical,
      load = res.cpu.load,
    },
    uptime = res.uptime,
    hostname = res.hostname,
  },
}))
"#;

    let value = run_lua_script(script);

    let flags = &value["flags"];
    assert_eq!(flags["windows"], Value::Bool(cfg!(target_os = "windows")));
    assert_eq!(flags["macos"], Value::Bool(cfg!(target_os = "macos")));
    assert_eq!(flags["linux"], Value::Bool(cfg!(target_os = "linux")));
    assert_eq!(
        flags["bsd"],
        Value::Bool(
            cfg!(target_os = "freebsd")
                || cfg!(target_os = "netbsd")
                || cfg!(target_os = "openbsd")
                || cfg!(target_os = "dragonfly")
        )
    );
    assert_eq!(flags["unix"], Value::Bool(cfg!(unix)));

    let info = &value["info"];
    assert_eq!(
        info["platform"],
        Value::from(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
    );
    assert_eq!(info["is_windows"], Value::Bool(cfg!(windows)));
    assert_eq!(info["is_unix"], Value::Bool(cfg!(unix)));

    let shell = &value["shell"];
    let expected_shell = if cfg!(windows) { "cmd" } else { "sh" };
    assert_eq!(shell[0], Value::from(expected_shell));
    let expected_arg = if cfg!(windows) { "/C" } else { "-lc" };
    assert_eq!(shell[1], Value::from(expected_arg));

    let resources = &value["resources"];
    let memory = &resources["memory"];
    let total = memory["total"].as_u64().expect("memory total");
    let available = memory["available"].as_u64().expect("memory available");
    let used = memory["used"].as_u64().expect("memory used");
    assert!(total >= available);
    assert!(used <= total);

    let cpu = &resources["cpu"];
    let logical = cpu["logical"].as_u64().expect("logical cores");
    assert!(logical > 0);
    let load = &cpu["load"];
    assert!(load["1m"].is_number());
    assert!(load["5m"].is_number());
    assert!(load["15m"].is_number());

    let uptime = resources["uptime"].as_u64().expect("uptime");
    assert!(uptime > 0);
    assert!(resources["hostname"].is_string());
}
