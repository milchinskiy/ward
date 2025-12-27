use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const CRYPTO_SCRIPT: &str = r#"local crypto = require("ward.crypto")
local json = require("ward.convert.json")

local payload = "The quick brown fox jumps over the lazy dog"
local file_path = _G.arg and _G.arg[1] or nil
local missing_path = _G.arg and _G.arg[2] or "missing.txt"

local hashes = {
  bytes = {
    sha256 = crypto.sha256(payload),
    sha1 = crypto.sha1(payload),
    md5 = crypto.md5(payload),
  },
  file = file_path and {
    sha256 = crypto.sha256_file(file_path),
    sha1 = crypto.sha1_file(file_path),
    md5 = crypto.md5_file(file_path),
  } or nil,
}

local missing_ok, missing_err = pcall(function()
  return crypto.sha256_file(missing_path)
end)

print(json.encode({
  hashes = hashes,
  missing_ok = missing_ok,
  missing_err = missing_err and tostring(missing_err) or nil,
}))
"#;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_crypto_script(temp: &TempDir, file_path: &Path, missing_path: &Path) -> Value {
    let script = write_script(temp, "crypto.lua", CRYPTO_SCRIPT);
    let output = ward_cmd()
        .args(["run", script.to_string_lossy().as_ref()])
        .arg(file_path)
        .arg(missing_path)
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
fn crypto_hashes_bytes_and_files() {
    let temp = tempdir().expect("tempdir");
    let payload_path = temp.path().join("payload.txt");
    let missing_path = temp.path().join("missing_payload.txt");
    let contents = "The quick brown fox jumps over the lazy dog";
    std::fs::write(&payload_path, contents).expect("write payload");

    let value = run_crypto_script(&temp, &payload_path, &missing_path);

    assert_eq!(
        value["hashes"]["bytes"]["sha256"],
        Value::from("d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592")
    );
    assert_eq!(
        value["hashes"]["bytes"]["sha1"],
        Value::from("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12")
    );
    assert_eq!(value["hashes"]["bytes"]["md5"], Value::from("9e107d9d372bb6826bd81d3542a419d6"));

    assert_eq!(
        value["hashes"]["file"]["sha256"],
        Value::from("d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592")
    );
    assert_eq!(
        value["hashes"]["file"]["sha1"],
        Value::from("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12")
    );
    assert_eq!(value["hashes"]["file"]["md5"], Value::from("9e107d9d372bb6826bd81d3542a419d6"));

    assert_eq!(value["missing_ok"], Value::Bool(false));
    let err = value["missing_err"].as_str().unwrap_or_default();
    assert!(!err.is_empty(), "expected missing file error");
}

#[test]
fn crypto_streams_large_files() {
    let temp = tempdir().expect("tempdir");
    let payload_path = temp.path().join("large.bin");
    let missing_path = temp.path().join("missing_large.bin");
    let contents = vec![b'a'; 200_000];
    std::fs::write(&payload_path, &contents).expect("write payload");

    let expected_sha256 = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        hex::encode(hasher.finalize())
    };
    let expected_sha1 = {
        use sha1::Digest;
        let mut hasher = sha1::Sha1::new();
        hasher.update(&contents);
        hex::encode(hasher.finalize())
    };
    let expected_md5 = format!("{:x}", md5::compute(&contents));

    let value = run_crypto_script(&temp, &payload_path, &missing_path);
    let file_hashes = &value["hashes"]["file"];

    assert_eq!(file_hashes["sha256"], Value::from(expected_sha256));
    assert_eq!(file_hashes["sha1"], Value::from(expected_sha1));
    assert_eq!(file_hashes["md5"], Value::from(expected_md5));

    assert_eq!(value["missing_ok"], Value::Bool(false));
    let err = value["missing_err"].as_str().unwrap_or_default();
    assert!(!err.is_empty(), "expected missing file error");
}
