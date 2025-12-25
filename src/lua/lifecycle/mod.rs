#![allow(clippy::missing_const_for_fn, clippy::needless_pass_by_value)]

use mlua::{AppDataRef, Function, Lua, RegistryKey, Table, Value};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering},
    },
};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy)]
pub enum ShutdownReason {
    Success,
    Error,
    Timeout,
    Signal,
    Requested,
}

impl ShutdownReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Signal => "signal",
            Self::Requested => "requested",
        }
    }
}

#[derive(Clone, Debug)]
struct SignalEvent {
    number: i32,
    name: &'static str,
}

fn signal_name_for(num: i32) -> &'static str {
    match num {
        1 => "HUP",
        2 => "INT",
        3 => "QUIT",
        10 => "USR1",
        12 => "USR2",
        13 => "PIPE",
        15 => "TERM",
        _ => "UNKNOWN",
    }
}

#[allow(clippy::cast_possible_truncation)]
fn parse_signal_spec(v: Value) -> mlua::Result<(i32, &'static str)> {
    match v {
        Value::Integer(i) => Ok((i as i32, signal_name_for(i as i32))),
        Value::Number(n) => Ok((n as i32, signal_name_for(n as i32))),
        Value::String(s) => {
            let up = s.to_str()?.to_ascii_uppercase();
            match up.as_str() {
                "HUP" => Ok((1, "HUP")),
                "INT" => Ok((2, "INT")),
                "QUIT" => Ok((3, "QUIT")),
                "USR1" => Ok((10, "USR1")),
                "USR2" => Ok((12, "USR2")),
                "PIPE" => Ok((13, "PIPE")),
                "TERM" => Ok((15, "TERM")),
                other => Err(mlua::Error::RuntimeError(format!("unknown signal: {other}"))),
            }
        }
        _ => Err(mlua::Error::RuntimeError(
            "signal must be a name (e.g. 'INT') or a number".into(),
        )),
    }
}

#[derive(Clone)]
struct LifecycleManager {
    // signal listeners start on first use
    started: Arc<AtomicBool>,

    // signal events flow into broadcast channel
    tx: broadcast::Sender<SignalEvent>,
    rx: Arc<Mutex<broadcast::Receiver<SignalEvent>>>,

    // handler ids
    next_id: Arc<AtomicU64>,
    // signal number -> handler ids
    sig_handlers: Arc<Mutex<HashMap<i32, Vec<u64>>>>,
    // shutdown handler ids (LIFO)
    shutdown_handlers: Arc<Mutex<Vec<u64>>>,

    // id -> handler record (holds RegistryKey; NOT cloned)
    ids: Arc<Mutex<HashMap<u64, HandlerRecord>>>,

    // shutdown request flag + suggested exit code
    shutdown_requested: Arc<AtomicBool>,
    shutdown_code: Arc<AtomicI32>,
    // shutdown origin (0 = unknown, 1 = requested, 2 = signal)
    shutdown_origin: Arc<AtomicU8>,

    // shutdown run-once
    shutdown_ran: Arc<AtomicBool>,
}

enum HandlerRecord {
    Signal { sig: i32, key: RegistryKey },
    Shutdown { key: RegistryKey },
}

impl LifecycleManager {
    fn new() -> Self {
        let (tx, rx0) = broadcast::channel(128);
        Self {
            started: Arc::new(AtomicBool::new(false)),
            tx,
            rx: Arc::new(Mutex::new(rx0)),
            next_id: Arc::new(AtomicU64::new(1)),
            sig_handlers: Arc::new(Mutex::new(HashMap::new())),
            shutdown_handlers: Arc::new(Mutex::new(Vec::new())),
            ids: Arc::new(Mutex::new(HashMap::new())),
            shutdown_requested: Arc::new(AtomicBool::new(false)),
            shutdown_code: Arc::new(AtomicI32::new(0)),
            shutdown_origin: Arc::new(AtomicU8::new(0)),
            shutdown_ran: Arc::new(AtomicBool::new(false)),
        }
    }

    fn ensure_started(&self) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }

        let tx = self.tx.clone();

        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            // Fixed superset: no dynamic mutation (safe + no deadlocks).
            let specs: &[(i32, &'static str, SignalKind)] = &[
                (1, "HUP", SignalKind::hangup()),
                (2, "INT", SignalKind::interrupt()),
                (3, "QUIT", SignalKind::quit()),
                (13, "PIPE", SignalKind::pipe()),
                (15, "TERM", SignalKind::terminate()),
                (10, "USR1", SignalKind::user_defined1()),
                (12, "USR2", SignalKind::user_defined2()),
            ];

            for (num, name, kind) in specs {
                let Ok(mut stream) = signal(*kind) else { continue };
                let tx2 = tx.clone();
                let ev = SignalEvent { number: *num, name };
                tokio::spawn(async move {
                    while stream.recv().await.is_some() {
                        let _ = tx2.send(ev.clone());
                    }
                });
            }
        }

        #[cfg(not(unix))]
        {
            // Windows: best-effort Ctrl-C -> INT
            let tx2 = tx.clone();
            tokio::spawn(async move {
                loop {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        let _ = tx2.send(SignalEvent { number: 2, name: "INT" });
                    }
                }
            });
        }
    }

    fn request_shutdown(&self, origin: ShutdownReason, code: Option<i32>) {
        self.shutdown_requested.store(true, Ordering::SeqCst);
        if let Some(c) = code {
            self.shutdown_code.store(c, Ordering::SeqCst);
        }

        let v = match origin {
            ShutdownReason::Requested => 1,
            ShutdownReason::Signal => 2,
            _ => 0,
        };
        if v != 0 {
            let _ = self.shutdown_origin.compare_exchange(0, v, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    fn requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::SeqCst)
    }

    fn code(&self) -> Option<i32> {
        if self.requested() {
            Some(self.shutdown_code.load(Ordering::SeqCst))
        } else {
            None
        }
    }

    fn origin(&self) -> Option<ShutdownReason> {
        if !self.requested() {
            return None;
        }
        match self.shutdown_origin.load(Ordering::SeqCst) {
            1 => Some(ShutdownReason::Requested),
            2 => Some(ShutdownReason::Signal),
            _ => None,
        }
    }
}

fn get_mgr(lua: &Lua) -> Option<AppDataRef<'_, LifecycleManager>> {
    lua.app_data_ref::<LifecycleManager>()
}

/// Called from your Lua VM hook (must be fast and non-blocking):
/// - drains pending signal events
/// - dispatches Lua callbacks
/// - requests shutdown on INT/TERM
/// - interrupts execution if shutdown is requested
/// # Errors [`mlua::Error`]
pub fn tick(lua: &Lua) -> mlua::Result<()> {
    let Some(mgr) = get_mgr(lua) else {
        return Ok(());
    };

    mgr.ensure_started();

    // Drain events without holding locks during Lua calls.
    let mut drained: Vec<SignalEvent> = Vec::new();
    {
        let mut rx = mgr
            .rx
            .lock()
            .map_err(|_| mlua::Error::RuntimeError("lifecycle receiver lock poisoned".into()))?;

        loop {
            match rx.try_recv() {
                Ok(ev) => drained.push(ev),
                Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => break,
                Err(broadcast::error::TryRecvError::Lagged(_)) => (),
            }
        }
    }

    for ev in drained {
        // Snapshot handler ids for this signal.
        let ids: Vec<u64> = {
            let map = mgr
                .sig_handlers
                .lock()
                .map_err(|_| mlua::Error::RuntimeError("lifecycle handlers lock poisoned".into()))?;
            map.get(&ev.number).cloned().unwrap_or_default()
        };

        // Snapshot functions (no RegistryKey cloning; build Functions while holding ids lock).
        let funcs: Vec<Function> = {
            let ids_map = mgr
                .ids
                .lock()
                .map_err(|_| mlua::Error::RuntimeError("lifecycle ids lock poisoned".into()))?;

            let mut out = Vec::new();
            for id in ids {
                if let Some(HandlerRecord::Signal { key, .. }) = ids_map.get(&id)
                    && let Ok(f) = lua.registry_value::<Function>(key)
                {
                    out.push(f);
                }
            }
            out
        };

        // Call handlers on Lua thread.
        for f in funcs {
            let payload = lua.create_table()?;
            payload.set("name", ev.name)?;
            payload.set("number", ev.number)?;
            // Best-effort: handler errors should not prevent shutdown path.
            let _ = f.call::<Value>(payload);
        }

        // Default behavior: INT/TERM requests shutdown.
        if ev.number == 2 || ev.number == 15 {
            mgr.request_shutdown(ShutdownReason::Signal, Some(128 + ev.number));
        }
    }

    if mgr.requested() {
        // Interrupt script execution so the runner can unwind and run shutdown callbacks.
        return Err(mlua::Error::external("interrupted"));
    }

    Ok(())
}

#[must_use]
pub fn shutdown_requested(lua: &Lua) -> bool {
    get_mgr(lua).is_some_and(|m| m.requested())
}

/// Request shutdown from Rust code (used by the runner for immediate Ctrl-C handling).
/// Safe to call even if lifecycle has not been initialized.
/// # Errors [`mlua::Error`]
pub fn request_shutdown(lua: &Lua, code: Option<i32>) -> mlua::Result<()> {
    let Some(mgr) = get_mgr(lua) else {
        return Ok(());
    };
    mgr.ensure_started();
    mgr.request_shutdown(ShutdownReason::Requested, code);
    Ok(())
}

/// Request shutdown because an external signal / Ctrl-C occurred.
/// # Errors [`mlua::Error`]
pub fn request_shutdown_signal(lua: &Lua, code: Option<i32>) -> mlua::Result<()> {
    let Some(mgr) = get_mgr(lua) else {
        return Ok(());
    };
    mgr.ensure_started();
    mgr.request_shutdown(ShutdownReason::Signal, code);
    Ok(())
}

#[must_use]
pub fn shutdown_origin(lua: &Lua) -> Option<ShutdownReason> {
    get_mgr(lua).and_then(|m| m.origin())
}

/// Run shutdown callbacks once (safe to call multiple times).
/// `error` is optional string context (e.g. script error message).
/// # Errors [`mlua::Error`]
pub fn run_shutdown(lua: &Lua, reason: ShutdownReason, error: Option<String>) -> mlua::Result<()> {
    let Some(mgr) = get_mgr(lua) else {
        return Ok(());
    };

    // run once
    if mgr.shutdown_ran.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let code = mgr.code();

    // Snapshot shutdown ids (LIFO order is applied later).
    let shutdown_ids: Vec<u64> = {
        let vec = mgr
            .shutdown_handlers
            .lock()
            .map_err(|_| mlua::Error::RuntimeError("shutdown handlers lock poisoned".into()))?;
        vec.clone()
    };

    // Execute in LIFO order. While doing so, we REMOVE handlers from ids map to obtain owned RegistryKey
    // (so we can call remove_registry_value without needing Clone).
    for id in shutdown_ids.into_iter().rev() {
        let rec = {
            let mut ids = mgr
                .ids
                .lock()
                .map_err(|_| mlua::Error::RuntimeError("lifecycle ids lock poisoned".into()))?;
            ids.remove(&id)
        };

        let Some(HandlerRecord::Shutdown { key }) = rec else {
            continue;
        };

        let f: Function = lua.registry_value(&key)?;
        let ctx = lua.create_table()?;
        ctx.set("reason", reason.as_str())?;
        ctx.set("code", code)?;
        ctx.set("error", error.clone())?;
        let _ = f.call::<Value>(ctx);

        let _ = lua.remove_registry_value(key);
    }

    // Clear shutdown list (best-effort).
    if let Ok(mut vec) = mgr.shutdown_handlers.lock() {
        vec.clear();
    }

    Ok(())
}

/// Initialize lifecycle module
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    if lua.app_data_ref::<LifecycleManager>().is_none() {
        lua.set_app_data(LifecycleManager::new());
    }

    let m = lua.create_table()?;

    // lifecycle.on(signal, fn) -> id
    m.set(
        "on",
        lua.create_function(|lua, (sigv, func): (Value, Function)| {
            let mgr = lua
                .app_data_ref::<LifecycleManager>()
                .ok_or_else(|| mlua::Error::RuntimeError("LifecycleManager missing".into()))?;
            mgr.ensure_started();

            let (num, _name) = parse_signal_spec(sigv)?;
            let id = mgr.next_id.fetch_add(1, Ordering::Relaxed);

            let key = lua.create_registry_value(func)?;

            {
                let mut map = mgr
                    .sig_handlers
                    .lock()
                    .map_err(|_| mlua::Error::RuntimeError("lifecycle handlers lock poisoned".into()))?;
                map.entry(num).or_default().push(id);
            }

            {
                let mut ids = mgr
                    .ids
                    .lock()
                    .map_err(|_| mlua::Error::RuntimeError("lifecycle ids lock poisoned".into()))?;
                ids.insert(id, HandlerRecord::Signal { sig: num, key });
            }
            drop(mgr);
            Ok(id)
        })?,
    )?;

    // lifecycle.on_shutdown(fn) -> id
    m.set(
        "on_shutdown",
        lua.create_function(|lua, func: Function| {
            let mgr = lua
                .app_data_ref::<LifecycleManager>()
                .ok_or_else(|| mlua::Error::RuntimeError("LifecycleManager missing".into()))?;

            let id = mgr.next_id.fetch_add(1, Ordering::Relaxed);
            let key = lua.create_registry_value(func)?;

            {
                let mut vec = mgr
                    .shutdown_handlers
                    .lock()
                    .map_err(|_| mlua::Error::RuntimeError("shutdown handlers lock poisoned".into()))?;
                vec.push(id);
            }

            {
                let mut ids = mgr
                    .ids
                    .lock()
                    .map_err(|_| mlua::Error::RuntimeError("lifecycle ids lock poisoned".into()))?;
                ids.insert(id, HandlerRecord::Shutdown { key });
            }
            drop(mgr);
            Ok(id)
        })?,
    )?;

    // lifecycle.off(id) -> boolean (works for both signal and shutdown handlers)
    m.set(
        "off",
        lua.create_function(|lua, id: u64| {
            let mgr = lua
                .app_data_ref::<LifecycleManager>()
                .ok_or_else(|| mlua::Error::RuntimeError("LifecycleManager missing".into()))?;

            let rec = {
                let mut ids = mgr
                    .ids
                    .lock()
                    .map_err(|_| mlua::Error::RuntimeError("lifecycle ids lock poisoned".into()))?;
                ids.remove(&id)
            };

            let Some(rec) = rec else {
                return Ok(false);
            };

            match rec {
                HandlerRecord::Signal { sig, key } => {
                    {
                        let mut map = mgr
                            .sig_handlers
                            .lock()
                            .map_err(|_| mlua::Error::RuntimeError("lifecycle handlers lock poisoned".into()))?;
                        if let Some(vec) = map.get_mut(&sig)
                            && let Some(pos) = vec.iter().position(|hid| *hid == id)
                        {
                            vec.remove(pos);
                        }
                    }
                    lua.remove_registry_value(key)?;
                    Ok(true)
                }
                HandlerRecord::Shutdown { key } => {
                    {
                        let mut vec = mgr
                            .shutdown_handlers
                            .lock()
                            .map_err(|_| mlua::Error::RuntimeError("shutdown handlers lock poisoned".into()))?;
                        if let Some(pos) = vec.iter().position(|hid| *hid == id) {
                            vec.remove(pos);
                        }
                    }
                    lua.remove_registry_value(key)?;
                    Ok(true)
                }
            }
        })?,
    )?;

    // lifecycle.request(code?) -> void
    m.set(
        "request",
        lua.create_function(|lua, code: Option<i32>| {
            let mgr = lua
                .app_data_ref::<LifecycleManager>()
                .ok_or_else(|| mlua::Error::RuntimeError("LifecycleManager missing".into()))?;
            mgr.request_shutdown(ShutdownReason::Requested, code);
            drop(mgr);
            Ok(())
        })?,
    )?;

    // lifecycle.requested() -> boolean
    m.set(
        "requested",
        lua.create_function(|lua, ()| {
            let mgr = lua
                .app_data_ref::<LifecycleManager>()
                .ok_or_else(|| mlua::Error::RuntimeError("LifecycleManager missing".into()))?;
            Ok(mgr.requested())
        })?,
    )?;

    // lifecycle.code() -> number|nil
    m.set(
        "code",
        lua.create_function(|lua, ()| {
            let mgr = lua
                .app_data_ref::<LifecycleManager>()
                .ok_or_else(|| mlua::Error::RuntimeError("LifecycleManager missing".into()))?;
            Ok(mgr.code())
        })?,
    )?;

    // lifecycle._tick() -> void  (optional; runner should call Rust tick() from hook)
    m.set(
        "_tick",
        lua.create_function(|lua, ()| {
            tick(lua)?;
            Ok(())
        })?,
    )?;

    // lifecycle._run_shutdown(reason?, error?) -> void (exposed mainly for testing)
    m.set(
        "_run_shutdown",
        lua.create_function(|lua, (reason, error): (Option<String>, Option<String>)| {
            let reason = match reason.as_deref() {
                Some("success") => ShutdownReason::Success,
                Some("timeout") => ShutdownReason::Timeout,
                Some("signal") => ShutdownReason::Signal,
                Some("requested") => ShutdownReason::Requested,
                Some("error") | None => ShutdownReason::Error,
                Some(other) => return Err(mlua::Error::RuntimeError(format!("unknown shutdown reason: {other}"))),
            };
            run_shutdown(lua, reason, error)?;
            Ok(())
        })?,
    )?;

    Ok(m)
}
