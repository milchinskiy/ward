use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tempfile::{TempDir, tempdir};

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script(name: &str, body: &str) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, name, body);

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
fn time_parsing_and_construction_cover_common_inputs() {
    let value = run_lua_script(
        "time_parse.lua",
        r#"local time = require("ward.time")
local json = require("ward.convert.json")

local parsed_rfc3339 = time.parse_rfc3339("2023-01-02T03:04:05Z")
local parsed_rfc3339_nil = time.parse_rfc3339("not-a-date")
local parsed_best = time.parse("2023-01-02 03:04:05")
local parsed_invalid = time.parse("bogus")
local parsed_with_format = time.parse("%Y/%m/%d %H:%M", "2023/01/02 03:04")
local from_ts = time.from_timestamp(-0.5)
local utc_tp = time.utc(2020, 2, 3, 4, 5, 6, 700000000)

print(json.encode({
  rfc3339_millis = parsed_rfc3339 and parsed_rfc3339:millis() or nil,
  rfc3339_invalid_is_nil = parsed_rfc3339_nil == nil,
  best_seconds = parsed_best and parsed_best:timestamp() or nil,
  parsed_invalid_is_nil = parsed_invalid == nil,
  fmt_iso_time = parsed_with_format and parsed_with_format:iso_time() or nil,
  from_ts_micros = from_ts:micros(),
  utc_iso_time = utc_tp:iso_time(),
}))
"#,
    );

    assert_eq!(value["rfc3339_millis"], Value::from(1_672_628_645_000i64));
    assert_eq!(value["rfc3339_invalid_is_nil"], Value::Bool(true));
    assert_eq!(value["parsed_invalid_is_nil"], Value::Bool(true));

    let best_seconds = value["best_seconds"].as_f64().expect("best_seconds");
    assert!((best_seconds - 1_672_628_645.0).abs() < f64::EPSILON);

    assert_eq!(value["fmt_iso_time"], Value::from("03:04:00"));
    assert_eq!(value["from_ts_micros"], Value::from(-500_000i64));
    assert_eq!(value["utc_iso_time"], Value::from("04:05:06.700000000"));
}

#[test]
fn duration_math_and_monotonic_elapsed_are_reported() {
    let value = run_lua_script(
        "time_duration.lua",
        r#"local time = require("ward.time")
local json = require("ward.convert.json")

local d_number = time.duration(1)
local d_string = time.duration("2500ms")

local sum = d_string + d_number
local diff = sum - d_string
local mul = d_string * 2
local neg_abs = d_number:neg():abs()

local instant = time.instant_now()
time.sleep("10ms"):wait()
local elapsed = instant:elapsed()

print(json.encode({
  number_micros = d_number:micros(),
  string_micros = d_string:micros(),
  sum_micros = sum:micros(),
  diff_micros = diff:micros(),
  mul_micros = mul:micros(),
  neg_abs_seconds = neg_abs:seconds(),
  elapsed_millis = elapsed:millis(),
}))
"#,
    );

    assert_eq!(value["number_micros"], Value::from(1_000_000i64));
    assert_eq!(value["string_micros"], Value::from(2_500_000i64));
    assert_eq!(value["sum_micros"], Value::from(3_500_000i64));
    assert_eq!(value["diff_micros"], Value::from(1_000_000i64));
    assert_eq!(value["mul_micros"], Value::from(5_000_000i64));

    let neg_abs_seconds = value["neg_abs_seconds"].as_f64().expect("neg_abs_seconds");
    assert!((neg_abs_seconds - 1.0).abs() < 1e-9);

    let elapsed_millis = value["elapsed_millis"].as_i64().expect("elapsed_millis");
    assert!(elapsed_millis >= 5, "elapsed_millis too small after sleep: {elapsed_millis}");
}

#[test]
fn timeout_interval_and_after_behaviors_match_contract() {
    let value = run_lua_script(
        "time_timers.lua",
        r#"local time = require("ward.time")
local json = require("ward.convert.json")

local timeout = time.timeout(time.sleep("50ms"), "5ms")
local timeout_ok, timeout_err = pcall(function()
  return timeout:wait()
end)

local interval = time.interval("0s")
local first_tick = interval:wait()
local second_tick = interval()
interval:reset()
local reset_tick = interval:wait()

local fired = false
local after = time.after("1ms", function()
  fired = true
  return "cb"
end)
local after_results = { after:wait() }

print(json.encode({
  timeout_ok = timeout_ok,
  timeout_err = timeout_err and tostring(timeout_err) or nil,
  interval_ticks = { first_tick, second_tick, reset_tick },
  after_results = after_results,
  after_fired = fired,
}))
"#,
    );

    assert_eq!(value["timeout_ok"], Value::Bool(false));
    let timeout_err = value["timeout_err"].as_str().unwrap_or_default();
    assert!(timeout_err.contains("timeout"), "unexpected timeout_err: {timeout_err}");

    let ticks = value["interval_ticks"].as_array().expect("interval_ticks");
    assert_eq!(ticks, &[Value::from(1), Value::from(2), Value::from(1)]);

    assert_eq!(value["after_results"][0], Value::from("cb"));
    assert_eq!(value["after_fired"], Value::Bool(true));
}
