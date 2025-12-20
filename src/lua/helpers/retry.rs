#![allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps, clippy::too_many_lines)]

use std::time::Duration;

use mlua::{Lua, Table, Value};
use rand::{Rng, SeedableRng};

#[derive(Clone, Copy)]
struct RetryOpts {
    attempts: u32,
    delay_ms: u64,
    backoff: f64,
    max_delay_ms: Option<u64>,
    jitter: bool,
    jitter_ratio: f64,
}

impl Default for RetryOpts {
    fn default() -> Self {
        Self {
            attempts: 3,
            delay_ms: 100,
            backoff: 2.0,
            max_delay_ms: None,
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
            if let Some(v) = t.get::<Option<u64>>("delay_ms")? {
                opts.delay_ms = v;
            }
            if let Some(v) = t.get::<Option<f64>>("backoff")? {
                opts.backoff = if v.is_finite() && v >= 1.0 { v } else { 1.0 };
            }
            opts.max_delay_ms = t.get::<Option<u64>>("max_delay_ms")?;
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
/// - `delay_ms` (default 100)
/// - `backoff` (default 2.0; minimum 1.0)
/// - `max_delay_ms`
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

            let mut delay_ms = opts.delay_ms;
            for attempt in 1..=opts.attempts {
                match func.call_async::<mlua::Value>(()).await {
                    Ok(v) => return Ok(v),
                    Err(e) => {
                        if attempt >= opts.attempts {
                            return Err(e);
                        }

                        let mut sleep_ms = delay_ms;
                        if let Some(max) = opts.max_delay_ms {
                            sleep_ms = sleep_ms.min(max);
                        }

                        if opts.jitter && sleep_ms > 0 {
                            let r = opts.jitter_ratio;
                            let lo = (1.0 - r).max(0.0);
                            let hi = 1.0 + r;
                            let factor: f64 = rng.random_range(lo..hi);
                            sleep_ms = ((sleep_ms as f64) * factor).round() as u64;
                        }

                        if sleep_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                        }

                        // Exponential backoff for next iteration
                        if opts.backoff > 1.0 && delay_ms > 0 {
                            delay_ms = ((delay_ms as f64) * opts.backoff).round() as u64;
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
