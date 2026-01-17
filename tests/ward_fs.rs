use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
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

fn to_set(value: &Value) -> HashSet<String> {
    value.as_array().map_or_else(HashSet::new, |arr| {
        arr.iter().filter_map(|v| v.as_str().map(ToOwned::to_owned)).collect()
    })
}

#[test]
fn fs_module_handles_common_operations_via_lua() {
    let script = r#"local fs = require("ward.fs")
local path = require("ward.fs.path")
local json = require("ward.convert.json")

local root = fs.tempdir("ward-fs-int")
local nested = fs.join(root, "alpha", "beta")
local mkdir_res = fs.mkdir(nested, { recursive = true, mode = 493, force = true })

local text_path = path.new(nested):join("note.txt")
local write_res = fs.write(text_path, "hello", { mode = "overwrite" })
local append_res = fs.write(text_path, " world", { mode = "append" })
local text_content = fs.read(text_path, { mode = "text" })

local binary_path = path.new(root):join("bytes.bin")
fs.write(binary_path, string.char(1, 2, 3, 4), { binary = true })
local binary_read = fs.read(binary_path, { mode = "binary" })
local binary_bytes = {}
for i = 1, #binary_read do
    binary_bytes[#binary_bytes + 1] = binary_read[i]
end

local unlink_res = fs.unlink(binary_path, { force = true })
local exists_after_unlink = fs.is_exists(binary_path)

local list_all = fs.list(root, { recursive = true })
local list_files = fs.list(root, { recursive = true, files = true, dirs = false })
local regex_list = fs.list(root, { recursive = true, regex = "note" })

local realpath = fs.realpath(text_path)
local dirname = fs.dirname(text_path:as_string())
local basename = fs.basename(text_path:as_string())
local normalized = path.new("a/../b/./c"):normalize():as_string()

local readable = fs.is_readable(text_path)
local writable_root = fs.is_writable(root)

print(json.encode({
  root = root,
  mkdir_ok = mkdir_res.ok,
  write_ok = write_res.ok,
  append_ok = append_res.ok,
  text = text_content,
  binary_bytes = binary_bytes,
  unlink_ok = unlink_res.ok,
  exists_after_unlink = exists_after_unlink,
  list_all = list_all,
  list_files = list_files,
  regex_list = regex_list,
  realpath = realpath,
  dirname = dirname,
  basename = basename,
  normalized = normalized,
  readable = readable,
  writable_root = writable_root,
}))
"#;

    let value = run_lua_script(script);

    assert_eq!(value["mkdir_ok"], Value::Bool(true));
    assert_eq!(value["write_ok"], Value::Bool(true));
    assert_eq!(value["append_ok"], Value::Bool(true));
    assert_eq!(value["text"], Value::from("hello world"));
    assert_eq!(
        value["binary_bytes"],
        Value::from(vec![Value::from(1), Value::from(2), Value::from(3), Value::from(4)])
    );
    assert_eq!(value["unlink_ok"], Value::Bool(true));
    assert_eq!(value["exists_after_unlink"], Value::Bool(false));

    let root = PathBuf::from(value["root"].as_str().expect("root path"));
    let alpha = root.join("alpha");
    let beta = alpha.join("beta");
    let text_path = beta.join("note.txt");

    let all_entries = to_set(&value["list_all"]);
    assert!(all_entries.contains(&alpha.to_string_lossy().into_owned()));
    assert!(all_entries.contains(&beta.to_string_lossy().into_owned()));
    assert!(all_entries.contains(&text_path.to_string_lossy().into_owned()));

    let file_entries = to_set(&value["list_files"]);
    assert_eq!(file_entries.len(), 1);
    assert!(file_entries.contains(&text_path.to_string_lossy().into_owned()));

    let regex_entries = to_set(&value["regex_list"]);
    assert_eq!(regex_entries, file_entries);

    let text_string = text_path.to_string_lossy().into_owned();
    assert_eq!(value["realpath"], Value::from(text_string));
    assert_eq!(value["dirname"], Value::from(beta.to_string_lossy().into_owned()));
    assert_eq!(value["basename"], Value::from("note.txt"));
    assert_eq!(
        value["normalized"],
        Value::from(PathBuf::from_iter(["b", "c"]).to_string_lossy().into_owned())
    );

    assert_eq!(value["readable"], Value::Bool(true));
    assert_eq!(value["writable_root"], Value::Bool(true));
}
