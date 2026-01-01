use hex::encode;
use serde_json::Value;
use sha2::{Digest, Sha256};
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

fn normalize_url(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_string()
}

fn store_id(url: &str, selector: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_url(url).as_bytes());
    hasher.update(b"\n");
    hasher.update(selector.as_bytes());
    encode(hasher.finalize())
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
    let expected_id = store_id(repo_dir.to_string_lossy().as_ref(), "head");
    let expected_store = externals_dir.join(".store").join(&expected_id);

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
  id = result.id,
  store_path = result.store_path,
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
    assert_eq!(value["id"], Value::from(expected_id.as_str()));
    assert_eq!(value["store_path"], Value::from(expected_store.to_string_lossy().as_ref()));
    assert_eq!(value["path"], Value::from(expected_store.to_string_lossy().as_ref()));
    assert_eq!(value["require_ok"], Value::Bool(true));
    assert_eq!(value["greet"], Value::from("hi"));

    let cloned_init = expected_store.join("init.lua");
    assert!(cloned_init.is_file(), "cloned module missing init.lua");
}

#[test]
#[allow(clippy::too_many_lines)]
fn module_git_allows_rebinding_with_force() {
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

    let init_lua_v1 = "return { greet = function() return \"v1\" end }\n";
    std::fs::write(repo_dir.join("init.lua"), init_lua_v1).expect("write init.lua");

    assert!(
        Command::new("git")
            .args(["add", "init.lua"])
            .current_dir(&repo_dir)
            .status()
            .expect("git add")
            .success()
    );

    let mut commit = Command::new("git");
    commit.args(["commit", "-m", "v1", "--quiet"]);
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

    let rev1 = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo_dir)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf8 rev1")
    .trim()
    .to_string();

    let init_lua_v2 = "return { greet = function() return \"v2\" end }\n";
    std::fs::write(repo_dir.join("init.lua"), init_lua_v2).expect("write init.lua v2");

    assert!(
        Command::new("git")
            .args(["add", "init.lua"])
            .current_dir(&repo_dir)
            .status()
            .expect("git add v2")
            .success()
    );

    let mut commit2 = Command::new("git");
    commit2.args(["commit", "-m", "v2", "--quiet"]);
    commit2.current_dir(&repo_dir);
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "Ward"),
        ("GIT_AUTHOR_EMAIL", "ward@example.com"),
        ("GIT_COMMITTER_NAME", "Ward"),
        ("GIT_COMMITTER_EMAIL", "ward@example.com"),
    ] {
        commit2.env(k, v);
    }
    assert!(commit2.status().expect("git commit v2").success());

    let rev2 = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo_dir)
            .output()
            .expect("rev-parse 2")
            .stdout,
    )
    .expect("utf8 rev2")
    .trim()
    .to_string();

    let data_home = temp.path().join("data_home");
    let externals_dir = data_home.join("ward").join("externals");
    let store_v1 = externals_dir
        .join(".store")
        .join(store_id(repo_dir.to_string_lossy().as_ref(), format!("rev:{rev1}").as_str()));
    let store_v2 = externals_dir
        .join(".store")
        .join(store_id(repo_dir.to_string_lossy().as_ref(), format!("rev:{rev2}").as_str()));

    let repo_literal = serde_json::to_string(repo_dir.to_string_lossy().as_ref()).expect("repo literal");
    let script = format!(
        r#"local module = require("ward.module")
local json = require("ward.convert.json")

local first = module.git({repo_literal}, {{ name = "foo", rev = "{rev1}" }})
local greet1 = require(first.require).greet()

local second_ok, second_err = pcall(function()
  return module.git({repo_literal}, {{ name = "foo", rev = "{rev2}" }})
end)

local forced = module.git({repo_literal}, {{ name = "foo", rev = "{rev2}", force = true }})
local greet2 = require(forced.require).greet()

print(json.encode({{
  first_id = first.id,
  forced_id = forced.id,
  second_ok = second_ok,
  second_err = tostring(second_err),
  greet1 = greet1,
  greet2 = greet2,
}}))
"#
    );

    let xdg_home = data_home.to_string_lossy();
    let value = run_lua_script_with_input(&script, &[("XDG_DATA_HOME", xdg_home.as_ref())]);

    assert_eq!(value["second_ok"], Value::Bool(false), "rebinding without force should fail");
    assert_eq!(value["greet1"], Value::from("v1"));
    assert_eq!(value["greet2"], Value::from("v2"));
    let first_id = value["first_id"].as_str().expect("first id");
    let forced_id = value["forced_id"].as_str().expect("forced id");
    assert_ne!(first_id, forced_id, "different revisions must map to different ids");
    assert_eq!(
        first_id,
        store_id(repo_dir.to_string_lossy().as_ref(), format!("rev:{rev1}").as_str())
    );
    assert_eq!(
        forced_id,
        store_id(repo_dir.to_string_lossy().as_ref(), format!("rev:{rev2}").as_str())
    );
    assert!(store_v1.is_dir(), "rev1 store missing");
    assert!(store_v2.is_dir(), "rev2 store missing");
}
