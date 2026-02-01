use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use tempfile::tempdir;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn apply_no_proxy(cmd: &mut Command) {
    for (k, v) in [
        ("NO_PROXY", "127.0.0.1,localhost"),
        ("no_proxy", "127.0.0.1,localhost"),
        ("HTTP_PROXY", ""),
        ("http_proxy", ""),
        ("HTTPS_PROXY", ""),
        ("https_proxy", ""),
    ] {
        cmd.env(k, v);
    }
}

fn run_lua_script(name: &str, body: &str) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, name, body);

    let mut cmd = ward_cmd();
    cmd.args(["run", script.to_string_lossy().as_ref()]);
    apply_no_proxy(&mut cmd);

    let output = cmd.output().expect("run output");

    assert!(
        output.status.success(),
        "lua script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("stdout json")
}

fn start_http_server<F>(handler: F) -> (SocketAddr, thread::JoinHandle<()>)
where
    F: Fn(&[u8]) -> Vec<u8> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 512];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut tmp).expect("read request");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > 8192 {
                    break;
                }
            }

            let response = handler(&buf);
            let _ = stream.write_all(&response);
        }
    });

    (addr, handle)
}

#[test]
fn http_get_returns_status_headers_and_body() {
    let (addr, server) = start_http_server(|req| {
        let request = String::from_utf8_lossy(req);
        assert!(request.starts_with("GET /hello HTTP/1.1"), "unexpected request: {request}");
        b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\nx-test: success\r\n\r\nhello world".to_vec()
    });

    let url = format!("http://{addr}/hello");
    let script = format!(
        r#"local http = require("ward.net.http")
local json = require("ward.convert.json")

local res = http.get({url}, {{ allow_error = true }})

print(json.encode({{
  status = res.status,
  ok = res:is_ok(),
  body = res.body,
  get_body = res:get_body(),
  get_status = res:get_status(),
  test_header = res.headers["x-test"],
  get_test_header = res:get_headers()["x-test"],
}}))
"#,
        url = serde_json::to_string(&url).expect("url literal")
    );

    let value = run_lua_script("net_http.lua", &script);
    server.join().expect("server thread");

    assert_eq!(value["status"], Value::from(200));
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["body"], Value::from("hello world"));
    assert_eq!(value["body"], value["get_body"]);
    assert_eq!(value["status"], value["get_status"]);
    assert_eq!(value["test_header"], Value::from("success"));
    assert_eq!(value["test_header"], value["get_test_header"]);
}

#[test]
fn fetch_url_honors_max_bytes_limit_and_cleans_target() {
    let (addr, server) = start_http_server(|req| {
        let request = String::from_utf8_lossy(req);
        assert!(request.starts_with("GET /file HTTP/1.1"), "unexpected request: {request}");
        b"HTTP/1.1 200 OK\r\ncontent-length: 6\r\n\r\nabcdef".to_vec()
    });

    let temp = tempdir().expect("tempdir");
    let dest_path = temp.path().join("fetched.bin");

    let url = format!("http://{addr}/file");
    let script = format!(
        r#"local fetch = require("ward.net.fetch")
local fs = require("ward.fs")
local json = require("ward.convert.json")

local dest = {dest}
local res = fetch.url({url}, {{ into = dest, max_bytes = 4 }})
local exists = fs.is_file(dest)

print(json.encode({{
  ok = res.ok,
  status = res.status,
  get_status = res:get_status(),
  path = res.path,
  get_path = res:get_path(),
  size = res.size,
  get_size = res:get_size(),
  file_exists = exists,
}}))
"#,
        url = serde_json::to_string(&url).expect("url literal"),
        dest = serde_json::to_string(&dest_path.to_string_lossy()).expect("dest literal"),
    );

    let value = run_lua_script("net_fetch.lua", &script);
    server.join().expect("server thread");

    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["status"], Value::from(413));
    assert_eq!(value["get_status"], Value::from(413));
    assert!(value["path"].is_null());
    assert!(value["get_path"].is_null());
    assert_eq!(value["size"], Value::from(0));
    assert_eq!(value["get_size"], Value::from(0));
    assert_eq!(value["file_exists"], Value::Bool(false));
    assert!(!dest_path.exists(), "fetch target should be removed");
}
