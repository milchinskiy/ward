#![allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps, clippy::too_many_lines)]

use std::time::Duration;

use mlua::{Lua, Table, Value};
use rand::{Rng, SeedableRng};
use serde_ext_duration::parse_str as parse_duration_str;

#[derive(Clone, Copy)]
struct RetryOpts {
    attempts: u32,
    delay: Duration,
    max_delay: Option<Duration>,
    backoff: f64,
    jitter: bool,
    jitter_ratio: f64,
}

impl Default for RetryOpts {
    fn default() -> Self {
        Self {
            attempts: 3,
            delay: Duration::from_millis(100),
            max_delay: None,
            backoff: 2.0,
            jitter: false,
            jitter_ratio: 0.2,
        }
    }
}

impl RetryOpts {
    fn from_value(value: Value) -> mlua::Result<Self> {
        let mut opts = Self::default();
        if let Value::Table(t) = value {
            if let Some(v) = t.get::<Option<u32>>("attempts")? {
                opts.attempts = v.max(1);
            }
            if let Some(v) = t.get::<Option<Value>>("delay")? {
                opts.delay = parse_duration_value(v, "delay")?;
            }
            if let Some(v) = t.get::<Option<f64>>("backoff")? {
                opts.backoff = if v.is_finite() && v >= 1.0 { v } else { 1.0 };
            }
            if let Some(v) = t.get::<Option<Value>>("max_delay")? {
                opts.max_delay = Some(parse_duration_value(v, "max_delay")?);
            }
            if let Some(v) = t.get::<Option<bool>>("jitter")? {
                opts.jitter = v;
            }
            if let Some(v) = t.get::<Option<f64>>("jitter_ratio")? {
                // Clamp into [0.0, 1.0]
                opts.jitter_ratio = v.clamp(0.0, 1.0);
            }
        }
        Ok(opts)
    }
}

/// Initializes the `helpers.retry` module.
///
/// API:
/// - `ward.helpers.retry.run(fn, opts?) -> any`
///   Retries `fn()` when it errors. The return value of `fn()` is returned on success.
///
/// Options (all optional):
/// - `attempts` (default 3)
/// - `delay` (default 100ms; accepts human-friendly duration strings)
/// - `max_delay`
/// - `backoff` (default 2.0; minimum 1.0)
/// - `jitter` (default false)
/// - `jitter_ratio` (default 0.2; range 0..1)
///
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "run",
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        lua.create_async_function(|_, (func, opts): (mlua::Function, Value)| async move {
            let opts = RetryOpts::from_value(opts)?;
            let mut rng = rand::rngs::SmallRng::from_os_rng();

            let mut delay = opts.delay;
            for attempt in 1..=opts.attempts {
                match func.call_async::<mlua::Value>(()).await {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        if attempt >= opts.attempts {
                            return Err(e);
                        }

                        let mut sleep_duration = delay;
                        if let Some(max) = opts.max_delay {
                            sleep_duration = sleep_duration.min(max);
                        }

                        if opts.jitter && !sleep_duration.is_zero() {
                            sleep_duration = apply_jitter(sleep_duration, opts.jitter_ratio, &mut rng);
                        }

                        if !sleep_duration.is_zero() {
                            tokio::time::sleep(sleep_duration).await;
                        }

                        // Exponential backoff for next iteration
                        if opts.backoff > 1.0 && !delay.is_zero() {
                            delay = scale_duration(delay, opts.backoff);
                        }
                    }
                }
            }

            // attempts is always >= 1
            Err(mlua::Error::external("retry: exhausted attempts"))
        })?,
    )?;

    Ok(t)
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn parse_duration_value(value: Value, field: &str) -> mlua::Result<Duration> {
    match value {
        Value::String(s) => {
            let text = s
                .to_str()
                .map_err(|_| mlua::Error::external(format!("{field} must be valid utf-8")))?;
            parse_duration_str(text.as_ref())
                .map_err(|_| mlua::Error::external(format!("failed to parse {field} duration")))
        }
        Value::Integer(i) => {
            if i.is_negative() {
                return Err(mlua::Error::external(format!("{field} must be non-negative")));
            }
            Ok(Duration::from_millis(i as u64))
        }
        Value::Number(n) => {
            if !n.is_finite() || n.is_sign_negative() {
                return Err(mlua::Error::external(format!("{field} must be a non-negative finite number")));
            }
            Ok(duration_from_millis(n))
        }
        other => Err(mlua::Error::external(format!(
            "{field} must be a string or number (got {other:?})"
        ))),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn duration_from_millis(millis: f64) -> Duration {
    if millis <= 0.0 || !millis.is_finite() {
        return Duration::ZERO;
    }

    #[allow(clippy::cast_precision_loss)]
    let clamped = if millis > u64::MAX as f64 {
        u64::MAX
    } else {
        millis.round() as u64
    };
    Duration::from_millis(clamped)
}

fn scale_duration(duration: Duration, factor: f64) -> Duration {
    if duration.is_zero() {
        return duration;
    }

    if !factor.is_finite() || factor <= 0.0 {
        return Duration::ZERO;
    }

    duration_from_millis(duration.as_secs_f64() * 1000.0 * factor)
}

fn apply_jitter(duration: Duration, ratio: f64, rng: &mut rand::rngs::SmallRng) -> Duration {
    if duration.is_zero() {
        return duration;
    }

    let r = ratio;
    let lo = (1.0 - r).max(0.0);
    let hi = 1.0 + r;
    let factor: f64 = rng.random_range(lo..hi);
    scale_duration(duration, factor)
}
