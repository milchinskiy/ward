#![allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]

use std::time::{Duration as StdDuration, Instant as StdInstant};

use chrono::{
    DateTime, Duration as ChronoDuration, FixedOffset, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc,
};
use mlua::{
    AnyUserData, Function, Lua, MetaMethod, MultiValue, ObjectLike, RegistryKey, Table, UserData, UserDataMethods,
    Value,
};

use chrono::{Datelike, Timelike};

#[derive(Clone, Debug)]
pub struct TimePoint(DateTime<Utc>);

impl TimePoint {
    #[must_use]
    pub const fn new(datetime: DateTime<Utc>) -> Self {
        Self(datetime)
    }

    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn timestamp_seconds(&self) -> f64 {
        // Keep float seconds for Lua ergonomics.
        // Note: f64 cannot precisely represent all nanoseconds for large timestamps.
        let nanos = f64::from(self.0.timestamp_subsec_nanos()) / 1_000_000_000.0;
        self.0.timestamp() as f64 + nanos
    }

    #[must_use]
    pub fn timestamp_millis(&self) -> i64 {
        self.0.timestamp_millis()
    }

    #[must_use]
    pub fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    #[must_use]
    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::AutoSi, true)
    }

    #[must_use]
    pub fn iso_date(&self) -> String {
        self.0.format("%Y-%m-%d").to_string()
    }

    #[must_use]
    pub fn iso_time(&self) -> String {
        let ns = self.0.nanosecond();
        if ns == 0 {
            self.0.format("%H:%M:%S").to_string()
        } else {
            // Always fixed 9 digits for stable parsing downstream.
            format!("{}.{:09}", self.0.format("%H:%M:%S"), self.0.nanosecond())
        }
    }

    #[must_use]
    pub fn format(&self, fmt: &str) -> String {
        self.0.format(fmt).to_string()
    }

    #[must_use]
    pub fn add(&self, duration: ChronoDuration) -> Self {
        Self(self.0 + duration)
    }

    #[must_use]
    pub fn sub_duration(&self, duration: ChronoDuration) -> Self {
        Self(self.0 - duration)
    }

    #[must_use]
    pub fn diff(&self, other: &Self) -> ChronoDuration {
        self.0 - other.0
    }

    #[must_use]
    pub fn to_parts(&self) -> (i32, u32, u32, u32, u32, u32, u32) {
        // (year, month, day, hour, minute, second, nanosecond)
        let dt = self.0;
        (
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
            dt.nanosecond(),
        )
    }
}

/// A signed time span.
#[derive(Clone, Debug)]
pub struct Duration(ChronoDuration);

impl Duration {
    #[must_use]
    pub fn new(d: ChronoDuration) -> Self {
        Self(d)
    }

    #[must_use]
    pub fn as_chrono(&self) -> ChronoDuration {
        self.0
    }

    #[allow(clippy::cast_precision_loss)]
    /// Returns the duration in seconds as a float.
    /// # Errors [`mlua::Error`]
    pub fn seconds_f64(&self) -> mlua::Result<f64> {
        let micros = self
            .0
            .num_microseconds()
            .ok_or_else(|| mlua::Error::external("duration overflow"))?;
        Ok(micros as f64 / 1_000_000.0)
    }

    #[must_use]
    pub fn millis_i64(&self) -> i64 {
        self.0.num_milliseconds()
    }

    /// Returns the duration in microseconds as an integer.
    /// # Errors [`mlua::Error`]
    pub fn micros_i64(&self) -> mlua::Result<i64> {
        self.0
            .num_microseconds()
            .ok_or_else(|| mlua::Error::external("duration overflow"))
    }

    #[must_use]
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    #[must_use]
    pub fn neg(&self) -> Self {
        Self(-self.0)
    }

    /// Multiplies the duration by a scalar.
    /// # Errors [`mlua::Error`]
    #[allow(clippy::cast_precision_loss)]
    pub fn mul_f64(&self, k: f64) -> mlua::Result<Self> {
        if !k.is_finite() {
            return Err(mlua::Error::external("multiplier must be finite"));
        }
        let micros = self
            .0
            .num_microseconds()
            .ok_or_else(|| mlua::Error::external("duration overflow"))?;
        let v = (micros as f64) * k;
        if v > i64::MAX as f64 || v < i64::MIN as f64 {
            return Err(mlua::Error::external("duration overflow"));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(Self(ChronoDuration::microseconds(v.round() as i64)))
    }
}

impl UserData for Duration {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("seconds", |_, this, ()| this.seconds_f64());
        methods.add_method("millis", |_, this, ()| Ok(this.millis_i64()));
        methods.add_method("micros", |_, this, ()| this.micros_i64());
        methods.add_method("abs", |_, this, ()| Ok(this.abs()));
        methods.add_method("neg", |_, this, ()| Ok(this.neg()));

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Duration({}ms)", this.millis_i64()))
        });
        methods.add_meta_method(MetaMethod::Eq, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(a.0 == b.0)
        });
        methods.add_meta_method(MetaMethod::Add, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(Self::new(a.0 + b.0))
        });
        methods.add_meta_method(MetaMethod::Sub, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(Self::new(a.0 - b.0))
        });
        methods.add_meta_method(MetaMethod::Mul, |_, a, k: f64| a.mul_f64(k));
    }
}

impl UserData for TimePoint {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Accessors
        methods.add_method("timestamp", |_, this, ()| Ok(this.timestamp_seconds()));
        methods.add_method("millis", |_, this, ()| Ok(this.timestamp_millis()));
        methods.add_method("micros", |_, this, ()| Ok(this.timestamp_micros()));
        methods.add_method("rfc3339", |_, this, ()| Ok(this.to_rfc3339()));
        methods.add_method("iso_date", |_, this, ()| Ok(this.iso_date()));
        methods.add_method("iso_time", |_, this, ()| Ok(this.iso_time()));
        methods.add_method("format", |_, this, fmt: String| Ok(this.format(&fmt)));

        // Operations
        methods.add_method("add", |_, this, delta: Value| {
            let duration = parse_duration(delta)?;
            Ok(this.add(duration.as_chrono()))
        });
        methods.add_method("sub", |_, this, delta: Value| {
            let duration = parse_duration(delta)?;
            Ok(this.sub_duration(duration.as_chrono()))
        });

        methods.add_method("diff", |lua, this, other: AnyUserData| {
            let other_tp = other.borrow::<Self>()?.clone();
            let d = this.diff(&other_tp);

            let res = lua.create_table()?;
            res.set("duration", lua.create_userdata(Duration::new(d))?)?;

            let micros = d
                .num_microseconds()
                .ok_or_else(|| mlua::Error::external("duration overflow"))?;
            #[allow(clippy::cast_precision_loss)]
            res.set("seconds", micros as f64 / 1_000_000.0)?;
            res.set("millis", d.num_milliseconds())?;
            Ok(res)
        });

        methods.add_method("parts", |lua, this, ()| {
            let (year, month, day, hour, minute, second, nanosecond) = this.to_parts();
            let t = lua.create_table()?;
            t.set("year", year)?;
            t.set("month", month)?;
            t.set("day", day)?;
            t.set("hour", hour)?;
            t.set("minute", minute)?;
            t.set("second", second)?;
            t.set("nanosecond", nanosecond)?;
            Ok(t)
        });

        // Metamethods
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.to_rfc3339()));
        methods.add_meta_method(MetaMethod::Eq, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(a.0 == b.0)
        });
        methods.add_meta_method(MetaMethod::Lt, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(a.0 < b.0)
        });
        methods.add_meta_method(MetaMethod::Le, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(a.0 <= b.0)
        });

        // timepoint + duration -> timepoint
        methods.add_meta_method(MetaMethod::Add, |_, tp, rhs: Value| {
            let d = parse_duration(rhs)?;
            Ok(tp.add(d.as_chrono()))
        });

        // timepoint - duration -> timepoint
        // timepoint - timepoint -> duration
        methods.add_meta_method(MetaMethod::Sub, |lua, tp, rhs: Value| {
            // Case 1: timepoint - duration => timepoint
            if let Ok(d) = parse_duration(rhs.clone()) {
                let out = tp.sub_duration(d.as_chrono());
                return Ok(Value::UserData(lua.create_userdata(out)?));
            }

            // Case 2: timepoint - timepoint => duration
            let Value::UserData(ud) = rhs else {
                return Err(mlua::Error::external("expected Duration or TimePoint"));
            };
            let other = ud.borrow::<Self>()?.clone();
            let d = tp.diff(&other);
            Ok(Value::UserData(lua.create_userdata(Duration::new(d))?))
        });
    }
}

/// A monotonic instant (not affected by wall-clock changes).
#[derive(Clone, Debug)]
pub struct InstantPoint(StdInstant);

impl UserData for InstantPoint {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("elapsed", |lua, this, ()| {
            let d = this.0.elapsed();
            lua.create_userdata(Duration::new(std_to_chrono(d)?))
        });

        methods.add_method("since", |lua, this, other: AnyUserData| {
            let other = other.borrow::<Self>()?.clone();
            let d = this
                .0
                .checked_duration_since(other.0)
                .ok_or_else(|| mlua::Error::external("instant is earlier than other"))?;
            lua.create_userdata(Duration::new(std_to_chrono(d)?))
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("InstantPoint(monotonic)".to_string()));

        // instant - instant -> duration
        methods.add_meta_method(MetaMethod::Sub, |lua, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?.clone();
            let d =
                a.0.checked_duration_since(b.0)
                    .ok_or_else(|| mlua::Error::external("instant is earlier than other"))?;
            Ok(Value::UserData(lua.create_userdata(Duration::new(std_to_chrono(d)?))?))
        });
    }
}

#[derive(Debug)]
struct SleepAwaitable {
    duration: StdDuration,
}

impl UserData for SleepAwaitable {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("wait", |_, this, ()| async move {
            tokio::time::sleep(this.duration).await;
            Ok(true)
        });
        methods.add_async_meta_method(MetaMethod::Call, |_, this, ()| async move {
            tokio::time::sleep(this.duration).await;
            Ok(true)
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Sleep({:?})", this.duration)));
    }
}

#[derive(Debug)]
struct AfterAwaitable {
    duration: StdDuration,
    cb_key: Option<RegistryKey>,
}

impl UserData for AfterAwaitable {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method_mut("wait", |lua, mut this, ()| async move {
            tokio::time::sleep(this.duration).await;
            call_optional_callback(&lua, &mut this).await
        });

        methods.add_async_meta_method_mut(MetaMethod::Call, |lua, mut this, ()| async move {
            tokio::time::sleep(this.duration).await;
            call_optional_callback(&lua, &mut this).await
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("After({:?})", this.duration)));
    }
}

async fn call_optional_callback(lua: &Lua, this: &mut AfterAwaitable) -> mlua::Result<MultiValue> {
    if let Some(key) = this.cb_key.take() {
        let f: mlua::Function = lua.registry_value(&key)?;
        lua.remove_registry_value(key)?;

        // Call callback and forward all return values.
        // (Supports both sync and async Lua functions under mlua's async feature.)
        let mv: MultiValue = f.call_async(()).await?;
        Ok(mv)
    } else {
        Ok(MultiValue::new())
    }
}

#[derive(Debug)]
struct TimeoutAwaitable {
    duration: StdDuration,
    inner_key: Option<RegistryKey>,
}

impl UserData for TimeoutAwaitable {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method_mut("wait", |lua, mut this, ()| async move { timeout_wait(&lua, &mut this).await });
        methods.add_async_meta_method_mut(MetaMethod::Call, |lua, mut this, ()| async move {
            timeout_wait(&lua, &mut this).await
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Timeout({:?})", this.duration)));
    }
}

async fn timeout_wait(lua: &Lua, this: &mut TimeoutAwaitable) -> mlua::Result<MultiValue> {
    let key = this
        .inner_key
        .take()
        .ok_or_else(|| mlua::Error::external("timeout awaitable already consumed"))?;

    let ud: AnyUserData = lua.registry_value(&key)?;

    let fut = async {
        // Prefer `wait()` for composability (e.g. time.timeout(time.sleep(...))).
        if let Ok(wait_fn) = ud.get::<Function>("wait") {
            return wait_fn.call_async::<MultiValue>((ud.clone(),)).await;
        }

        if let Ok(call_fn) = ud.get::<Function>("__call") {
            return call_fn.call_async::<MultiValue>((ud.clone(),)).await;
        }

        Err(mlua::Error::external("awaitable has neither wait() nor __call()"))
    };

    let res = tokio::time::timeout(this.duration, fut).await;

    // Always release registry.
    lua.remove_registry_value(key)?;

    res.unwrap_or_else(|_| Err(mlua::Error::external("timeout")))
}

#[derive(Debug)]
struct IntervalTimer {
    period: StdDuration,
    next_deadline: tokio::time::Instant,
    tick: u64,
}

impl UserData for IntervalTimer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("period", |lua, this, ()| {
            lua.create_userdata(Duration::new(std_to_chrono(this.period)?))
        });

        methods.add_method_mut("reset", |_, this, ()| {
            this.tick = 0;
            this.next_deadline = tokio::time::Instant::now() + this.period;
            Ok(true)
        });

        methods.add_async_method_mut("wait", |_, mut this, ()| async move {
            tokio::time::sleep_until(this.next_deadline).await;
            this.tick = this.tick.saturating_add(1);
            this.next_deadline = this.next_deadline + this.period;
            Ok(this.tick)
        });

        methods.add_async_meta_method_mut(MetaMethod::Call, |_, mut this, ()| async move {
            tokio::time::sleep_until(this.next_deadline).await;
            this.tick = this.tick.saturating_add(1);
            this.next_deadline = this.next_deadline + this.period;
            Ok(this.tick)
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Interval({:?})", this.period)));
    }
}

/// Returns a table with time functions
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let time = lua.create_table()?;

    // Wall clock
    time.set("now", lua.create_function(|_, ()| Ok(TimePoint::new(Utc::now())))?)?;
    time.set(
        "now_table",
        lua.create_function(|lua, ()| {
            let tp = TimePoint::new(Utc::now());
            let t = lua.create_table()?;
            t.set("tp", lua.create_userdata(tp.clone())?)?;
            t.set("timestamp", tp.timestamp_seconds())?;
            t.set("millis", tp.timestamp_millis())?;
            t.set("micros", tp.timestamp_micros())?;
            t.set("rfc3339", tp.to_rfc3339())?;
            Ok(t)
        })?,
    )?;

    time.set("parse_rfc3339", lua.create_function(|_, value: String| parse_rfc3339(&value))?)?;
    time.set("parse_rfc2822", lua.create_function(|_, value: String| parse_rfc2822(&value))?)?;
    time.set(
        "parse",
        lua.create_function(|_, (fmt, value): (String, String)| parse_with_format(&fmt, &value))?,
    )?;

    time.set(
        "from_timestamp",
        lua.create_function(|_, seconds: f64| from_timestamp(seconds))?,
    )?;

    time.set(
        "utc",
        lua.create_function(
            |_, (year, month, day, hour, minute, second, nanosecond): (i32, u32, u32, u32, u32, u32, Option<u32>)| {
                let ns = nanosecond.unwrap_or(0);
                let dt = Utc
                    .with_ymd_and_hms(year, month, day, hour, minute, second)
                    .single()
                    .ok_or_else(|| mlua::Error::external("invalid date/time"))?
                    .with_nanosecond(ns)
                    .ok_or_else(|| mlua::Error::external("invalid nanosecond"))?;
                Ok(TimePoint::new(dt))
            },
        )?,
    )?;

    // Duration helpers
    time.set("duration", lua.create_function(|_, v: Value| parse_duration(v))?)?;

    // Monotonic time
    time.set("instant_now", lua.create_function(|_, ()| Ok(InstantPoint(StdInstant::now())))?)?;

    // Awaitable timers
    time.set(
        "sleep",
        lua.create_function(|lua, v: Value| {
            let d = parse_duration(v)?;
            let std = chrono_to_std_nonneg(d.as_chrono())?;
            lua.create_userdata(SleepAwaitable { duration: std })
        })?,
    )?;

    time.set(
        "after",
        lua.create_function(|lua, (v, cb): (Value, Option<mlua::Function>)| {
            let d = parse_duration(v)?;
            let std = chrono_to_std_nonneg(d.as_chrono())?;

            let cb_key = match cb {
                Some(f) => Some(lua.create_registry_value(f)?),
                None => None,
            };

            lua.create_userdata(AfterAwaitable { duration: std, cb_key })
        })?,
    )?;

    time.set(
        "interval",
        lua.create_function(|lua, v: Value| {
            let d = parse_duration(v)?;
            let std = chrono_to_std_nonneg(d.as_chrono())?;
            let now = tokio::time::Instant::now();
            lua.create_userdata(IntervalTimer {
                period: std,
                next_deadline: now + std,
                tick: 0,
            })
        })?,
    )?;

    time.set(
        "timeout",
        lua.create_function(|lua, (awaitable, v): (AnyUserData, Value)| {
            let d = parse_duration(v)?;
            let std = chrono_to_std_nonneg(d.as_chrono())?;
            let key = lua.create_registry_value(awaitable)?;
            lua.create_userdata(TimeoutAwaitable {
                duration: std,
                inner_key: Some(key),
            })
        })?,
    )?;

    // Back-compat blocking sleep
    time.set(
        "sleep_blocking",
        lua.create_function(|_, seconds: f64| {
            validate_sleep(seconds)?;
            std::thread::sleep(StdDuration::from_secs_f64(seconds));
            Ok(true)
        })?,
    )?;

    Ok(time)
}

fn validate_sleep(seconds: f64) -> mlua::Result<()> {
    if seconds.is_sign_negative() {
        return Err(mlua::Error::external("sleep duration must be non-negative"));
    }
    if !seconds.is_finite() {
        return Err(mlua::Error::external("sleep duration must be finite"));
    }
    Ok(())
}

fn parse_rfc3339(value: &str) -> mlua::Result<TimePoint> {
    let datetime = DateTime::parse_from_rfc3339(value)
        .map_err(mlua::Error::external)?
        .with_timezone(&Utc);
    Ok(TimePoint::new(datetime))
}

fn parse_rfc2822(value: &str) -> mlua::Result<TimePoint> {
    let datetime = DateTime::parse_from_rfc2822(value)
        .map_err(mlua::Error::external)?
        .with_timezone(&Utc);
    Ok(TimePoint::new(datetime))
}

/// Parse using chrono format strings.
/// # Errors [`mlua::Error`]
fn parse_with_format(fmt: &str, value: &str) -> mlua::Result<TimePoint> {
    if let Ok(dt) = DateTime::<FixedOffset>::parse_from_str(value, fmt) {
        return Ok(TimePoint::new(dt.with_timezone(&Utc)));
    }

    if let Ok(naive_dt) = NaiveDateTime::parse_from_str(value, fmt) {
        let dt = Utc.from_utc_datetime(&naive_dt);
        return Ok(TimePoint::new(dt));
    }

    if let Ok(naive_d) = NaiveDate::parse_from_str(value, fmt) {
        let naive_dt = naive_d
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| mlua::Error::external("invalid date"))?;
        let dt = Utc.from_utc_datetime(&naive_dt);
        return Ok(TimePoint::new(dt));
    }

    Err(mlua::Error::external("failed to parse date/time"))
}

/// Safer f64 seconds -> `DateTime`, including negative timestamps.
#[allow(clippy::cast_possible_truncation)]
fn from_timestamp(seconds: f64) -> mlua::Result<TimePoint> {
    if !seconds.is_finite() {
        return Err(mlua::Error::external("timestamp must be finite"));
    }

    // For negative values, trunc() would move toward 0, which breaks the fraction.
    let secs_f = seconds.floor();
    let mut secs = secs_f as i64;
    let frac = seconds - secs_f; // [0, 1)
    let mut nanos = (frac * 1_000_000_000.0).round() as i64;

    // normalize rounding edge
    if nanos >= 1_000_000_000 {
        secs = secs.saturating_add(1);
        nanos -= 1_000_000_000;
    }

    let nanos_u32 = u32::try_from(nanos).map_err(|_| mlua::Error::external("timestamp nanoseconds out of range"))?;

    let datetime = Utc
        .timestamp_opt(secs, nanos_u32)
        .single()
        .ok_or_else(|| mlua::Error::external("timestamp is out of range"))?;
    Ok(TimePoint::new(datetime))
}

/// Accepts:
/// - number: seconds (f64)
/// - table: { days/hours/minutes/seconds/millis/micros }
/// - Duration userdata
fn parse_duration(value: Value) -> mlua::Result<Duration> {
    match value {
        Value::Number(n) => duration_from_seconds(n).map(Duration::new),
        #[allow(clippy::cast_precision_loss)]
        Value::Integer(i) => duration_from_seconds(i as f64).map(Duration::new),
        Value::Table(t) => {
            let days = t.get::<Option<f64>>("days")?.unwrap_or(0.0);
            let hours = t.get::<Option<f64>>("hours")?.unwrap_or(0.0);
            let minutes = t.get::<Option<f64>>("minutes")?.unwrap_or(0.0);
            let seconds = t.get::<Option<f64>>("seconds")?.unwrap_or(0.0);
            let millis = t.get::<Option<f64>>("millis")?.unwrap_or(0.0);
            let micros = t.get::<Option<f64>>("micros")?.unwrap_or(0.0);

            let total_seconds = seconds
                + (minutes * 60.0)
                + (hours * 3_600.0)
                + (days * 86_400.0)
                + (millis / 1_000.0)
                + (micros / 1_000_000.0);

            duration_from_seconds(total_seconds).map(Duration::new)
        }
        Value::UserData(ud) => ud.borrow::<Duration>().map_or_else(
            |_| {
                Err(mlua::Error::external(
                    "userdata is not a Duration (expected number, table, or Duration)",
                ))
            },
            |d| Ok(d.clone()),
        ),
        _ => Err(mlua::Error::external("duration must be number, table, or Duration userdata")),
    }
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn duration_from_seconds(seconds: f64) -> mlua::Result<ChronoDuration> {
    if !seconds.is_finite() {
        return Err(mlua::Error::external("duration must be finite"));
    }

    let micros = seconds * 1_000_000.0;
    if micros > i64::MAX as f64 || micros < i64::MIN as f64 {
        return Err(mlua::Error::external("duration is out of range"));
    }

    Ok(ChronoDuration::microseconds(micros.round() as i64))
}

fn chrono_to_std_nonneg(d: ChronoDuration) -> mlua::Result<StdDuration> {
    let micros = d
        .num_microseconds()
        .ok_or_else(|| mlua::Error::external("duration overflow"))?;
    if micros < 0 {
        return Err(mlua::Error::external("duration must be non-negative"));
    }
    let umicros = u64::try_from(micros).map_err(|_| mlua::Error::external("duration overflow"))?;
    Ok(StdDuration::from_micros(umicros))
}

fn std_to_chrono(d: StdDuration) -> mlua::Result<ChronoDuration> {
    // Convert with microsecond precision.
    let micros = d.as_micros();
    let micros_i64 = i64::try_from(micros).map_err(|_| mlua::Error::external("duration overflow"))?;
    Ok(ChronoDuration::microseconds(micros_i64))
}
