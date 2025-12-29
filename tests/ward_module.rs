use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script_with_input(body: &str, envs: &[(&str, &str)]) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, "script.lua", body);

    let mut cmd = ward_cmd();
    cmd.args(["run", script.to_string_lossy().as_ref()]);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().expect("run output");

    assert!(
        output.status.success(),
        "lua script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("stdout json")
}

#[test]
fn module_git_clones_local_repo_and_reports_paths() {
    let temp = tempdir().expect("tempdir");
    let repo_dir = temp.path().join("repo");
    std::fs::create_dir_all(&repo_dir).expect("repo dir");

    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repo_dir)
            .status()
            .expect("git init")
            .success()
    );

    let init_lua = "return { greet = function() return \"hi\" end }\n";
    std::fs::write(repo_dir.join("init.lua"), init_lua).expect("write init.lua");

    assert!(
        Command::new("git")
            .args(["add", "init.lua"])
            .current_dir(&repo_dir)
            .status()
            .expect("git add")
            .success()
    );

    let mut commit = Command::new("git");
    commit.args(["commit", "-m", "init", "--quiet"]);
    commit.current_dir(&repo_dir);
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "Ward"),
        ("GIT_AUTHOR_EMAIL", "ward@example.com"),
        ("GIT_COMMITTER_NAME", "Ward"),
        ("GIT_COMMITTER_EMAIL", "ward@example.com"),
    ] {
        commit.env(k, v);
    }

    assert!(commit.status().expect("git commit").success());

    let data_home = temp.path().join("data_home");
    let externals_dir = data_home.join("ward").join("externals");

    let repo_literal = serde_json::to_string(repo_dir.to_string_lossy().as_ref()).expect("repo literal");
    let script = format!(
        r#"local module = require("ward.module")
local json = require("ward.convert.json")

local result = module.git({repo_literal}, {{ name = "My Repo", force = true }})
local require_ok, required = pcall(function()
  local mod = require(result.require)
  return mod.greet()
end)

print(json.encode({{
  dir = module.dir(),
  ok = result.ok,
  name = result.name,
  require = result.require,
  path = result.path,
  require_ok = require_ok,
  greet = required,
}}))
"#
    );

    let xdg_home = data_home.to_string_lossy();
    let value = run_lua_script_with_input(&script, &[("XDG_DATA_HOME", xdg_home.as_ref())]);

    let expected_name = Value::from("my_repo");
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["name"], expected_name);
    assert_eq!(value["require"], Value::from("externals.my_repo"));
    assert_eq!(value["dir"], Value::from(externals_dir.to_string_lossy().as_ref()));
    assert_eq!(
        value["path"],
        Value::from(externals_dir.join("my_repo").to_string_lossy().as_ref())
    );
    assert_eq!(value["require_ok"], Value::Bool(true));
    assert_eq!(value["greet"], Value::from("hi"));

    let cloned_init = externals_dir.join("my_repo").join("init.lua");
    assert!(cloned_init.is_file(), "cloned module missing init.lua");
}
