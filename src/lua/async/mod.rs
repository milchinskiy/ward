#![allow(clippy::unnecessary_wraps)]

use mlua::{
    AnyUserData, Function, Lua, MetaMethod, MultiValue, ObjectLike, RegistryKey, Table, UserData, UserDataMethods,
    Value, Variadic,
};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot};

#[derive(Debug)]
struct Task {
    rx: Option<oneshot::Receiver<mlua::Result<RegistryKey>>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Task {
    fn drop(&mut self) {
        // Structured concurrency default: if user drops the handle,
        // the task should not outlive the script.
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

impl UserData for Task {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("done", |_, this, ()| {
            Ok(this.handle.as_ref().is_some_and(tokio::task::JoinHandle::is_finished))
        });

        methods.add_method_mut("cancel", |_, this, ()| {
            if let Some(h) = this.handle.take() {
                h.abort();
            }
            // Drop receiver so the producer can clean up if it completes.
            this.rx.take();
            Ok(true)
        });

        methods.add_async_method_mut("join", |lua, mut this, ()| async move {
            let rx = this
                .rx
                .take()
                .ok_or_else(|| mlua::Error::external("task already joined"))?;

            // Await completion.
            let Ok(res) = rx.await else {
                // Sender dropped without sending: cancelled/aborted.
                return Err(mlua::Error::external("cancelled"));
            };

            // The join handle may still exist if the producer finished quickly;
            // we can drop it now.
            this.handle.take();

            let key = res?;

            // Materialize return values from registry table.
            let t: mlua::Table = lua.registry_value(&key)?;
            lua.remove_registry_value(key)?;

            let mut mv = MultiValue::new();
            let len = i64::try_from(t.raw_len()).map_err(|_| mlua::Error::external("too many return values"))?;
            for i in 1..=len {
                mv.push_back(t.raw_get::<Value>(i)?);
            }
            Ok(mv)
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("Task()".to_string()));
    }
}

#[derive(Debug)]
struct ChannelInner {
    // tx is accessed from sync methods; keep std::sync::Mutex.
    tx: StdMutex<Option<mpsc::Sender<RegistryKey>>>,
    // rx must support concurrent recv() calls; serialize them with an async mutex.
    rx: TokioMutex<mpsc::Receiver<RegistryKey>>,
}

#[derive(Debug)]
struct Channel {
    lua: Lua,
    inner: Arc<ChannelInner>,
}

impl Channel {
    fn drain_registry_queue(&self) {
        // Best-effort cleanup: remove any queued registry values so they do not leak.
        // Cannot await here; use try_lock best-effort.
        let Ok(mut rx) = self.inner.rx.try_lock() else {
            return;
        };
        while let Ok(key) = rx.try_recv() {
            let _ = self.lua.remove_registry_value(key);
        }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        if let Ok(mut txg) = self.inner.tx.lock() {
            *txg = None;
        }
        self.drain_registry_queue();
    }
}

impl UserData for Channel {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("send", |lua, this, v: Value| {
            // Clone sender so we do not borrow the userdata across await.
            let tx = this.inner.tx.lock().map_or_else(|_| None, |g| g.clone());
            async move {
                let closed = Value::String(lua.create_string("closed")?);

                let Some(tx) = tx else {
                    return Ok(mv2(&lua, Value::Nil, closed));
                };

                let key = lua.create_registry_value(v)?;
                match tx.send(key).await {
                    Ok(()) => Ok(mv1(&lua, Value::Boolean(true))),
                    Err(e) => {
                        lua.remove_registry_value(e.0)?;
                        Ok(mv2(&lua, Value::Nil, closed))
                    }
                }
            }
        });

        methods.add_method_mut("try_send", |lua, this, v: Value| {
            let tx = this
                .inner
                .tx
                .lock()
                .map_err(|_| mlua::Error::external("channel mutex poisoned"))?
                .clone();
            let Some(tx) = tx.as_ref() else {
                return Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("closed")?)));
            };

            let key = lua.create_registry_value(v)?;
            match tx.clone().try_send(key) {
                Ok(()) => Ok(mv1(lua, Value::Boolean(true))),
                Err(mpsc::error::TrySendError::Full(k)) => {
                    lua.remove_registry_value(k)?;
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("full")?)))
                }
                Err(mpsc::error::TrySendError::Closed(k)) => {
                    lua.remove_registry_value(k)?;
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("closed")?)))
                }
            }
        });

        methods.add_async_method("recv", |lua, this, ()| {
            let inner = this.inner.clone();
            async move {
                // Serialize recv() across multiple consumers via async mutex.
                let mut rx = inner.rx.lock().await;
                let msg = rx.recv().await;
                drop(rx);

                match msg {
                    Some(key) => {
                        let v: Value = lua.registry_value(&key)?;
                        lua.remove_registry_value(key)?;
                        Ok(mv1(&lua, v))
                    }
                    None => Ok(mv2(&lua, Value::Nil, Value::String(lua.create_string("closed")?))),
                }
            }
        });

        methods.add_method("try_recv", |lua, this, ()| {
            let Ok(mut rx) = this.inner.rx.try_lock() else {
                // Someone is currently awaiting recv(); treat as "empty" (or "busy" if you prefer).
                return Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("empty")?)));
            };
            match rx.try_recv() {
                Ok(key) => {
                    let v: Value = lua.registry_value(&key)?;
                    lua.remove_registry_value(key)?;
                    Ok(mv1(lua, v))
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("empty")?)))
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("closed")?)))
                }
            }
        });

        methods.add_method_mut("close", |_, this, ()| {
            if let Ok(mut txg) = this.inner.tx.lock() {
                *txg = None;
            }
            Ok(true)
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("Channel()".to_string()));
    }
}

fn mv1(_lua: &Lua, a: Value) -> MultiValue {
    let mut mv = MultiValue::new();
    mv.push_back(a);
    mv
}

fn mv2(_lua: &Lua, a: Value, b: Value) -> MultiValue {
    let mut mv = MultiValue::new();
    mv.push_back(a);
    mv.push_back(b);
    mv
}

fn parse_capacity(v: Value) -> mlua::Result<usize> {
    match v {
        Value::Nil => Ok(64),
        Value::Integer(i) => {
            if i <= 0 {
                return Err(mlua::Error::external("capacity must be positive"));
            }
            #[allow(clippy::cast_sign_loss)]
            Ok(usize::try_from(i).map_err(|_| mlua::Error::external("capacity overflow"))?)
        }
        #[allow(clippy::cast_precision_loss)]
        Value::Number(n) => {
            if !n.is_finite() || n <= 0.0 {
                return Err(mlua::Error::external("capacity must be positive"));
            }
            if n > (usize::MAX as f64) {
                return Err(mlua::Error::external("capacity overflow"));
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(n as usize)
        }
        Value::Table(t) => {
            let cap = t.get::<Option<Value>>("capacity")?.unwrap_or(Value::Nil);
            parse_capacity(cap)
        }
        _ => Err(mlua::Error::external("channel expects opts table or capacity number")),
    }
}

/// Returns a table with async primitives.
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    // async.spawn(fn, ...) -> Task
    t.set(
        "spawn",
        lua.create_function(|lua, (f, args): (mlua::Function, Variadic<Value>)| {
            // Store function and args in registry so we don't hold Lua values across threads.
            let f_key = lua.create_registry_value(f)?;
            let args_table = lua.create_table()?;
            for (i, v) in args.into_iter().enumerate() {
                // Lua arrays are 1-based
                args_table.raw_set((i + 1) as i64, v)?;
            }
            let args_key = lua.create_registry_value(args_table)?;

            let (tx, rx) = oneshot::channel::<mlua::Result<RegistryKey>>();
            let lua2 = lua.clone();

            let handle = tokio::task::spawn_local(async move {
                // Ensure we always release registry keys (move-only) on any path.
                struct Guard {
                    lua: Lua,
                    f_key: Option<RegistryKey>,
                    args_key: Option<RegistryKey>,
                }
                impl Drop for Guard {
                    fn drop(&mut self) {
                        if let Some(k) = self.f_key.take() {
                            let _ = self.lua.remove_registry_value(k);
                        }
                        if let Some(k) = self.args_key.take() {
                            let _ = self.lua.remove_registry_value(k);
                        }
                    }
                }

                let mut guard = Guard {
                    lua: lua2.clone(),
                    f_key: Some(f_key),
                    args_key: Some(args_key),
                };

                let fut = async {
                    let f_key_ref = guard
                        .f_key
                        .as_ref()
                        .ok_or_else(|| mlua::Error::external("internal: missing f_key"))?;
                    let args_key_ref = guard
                        .args_key
                        .as_ref()
                        .ok_or_else(|| mlua::Error::external("internal: missing args_key"))?;

                    let f: mlua::Function = lua2.registry_value(f_key_ref)?;
                    let args_table: mlua::Table = lua2.registry_value(args_key_ref)?;

                    // Release stored inputs as soon as possible (consume keys).
                    if let Some(k) = guard.f_key.take() {
                        lua2.remove_registry_value(k)?;
                    }
                    if let Some(k) = guard.args_key.take() {
                        lua2.remove_registry_value(k)?;
                    }

                    // Materialize args into MultiValue.
                    let mut mv = MultiValue::new();
                    let len =
                        i64::try_from(args_table.raw_len()).map_err(|_| mlua::Error::external("too many arguments"))?;
                    for i in 1..=len {
                        mv.push_back(args_table.raw_get::<Value>(i)?);
                    }

                    let out: MultiValue = f.call_async(mv).await?;

                    // Store results in a registry table.
                    let res_table = lua2.create_table()?;
                    for (i, v) in out.into_iter().enumerate() {
                        res_table.raw_set((i + 1) as i64, v)?;
                    }
                    let res_key = lua2.create_registry_value(res_table)?;
                    Ok::<RegistryKey, mlua::Error>(res_key)
                };

                let r = fut.await;

                // If the join handle is dropped, `tx.send` will fail; clean up registry then.
                match r {
                    Ok(key) => {
                        // If sending fails, we get the value back and must clean it up.
                        if let Err(v) = tx.send(Ok(key))
                            && let Ok(key) = v
                        {
                            let _ = lua2.remove_registry_value(key);
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            });

            lua.create_userdata(Task {
                rx: Some(rx),
                handle: Some(handle),
            })
        })?,
    )?;

    // async.channel(opts) -> Channel
    t.set(
        "channel",
        lua.create_function(|lua, opts: Value| {
            let cap = parse_capacity(opts)?;
            let (tx, rx) = mpsc::channel::<RegistryKey>(cap);
            lua.create_userdata(Channel {
                lua: lua.clone(),
                inner: Arc::new(ChannelInner {
                    tx: StdMutex::new(Some(tx)),
                    rx: TokioMutex::new(rx),
                }),
            })
        })?,
    )?;

    // Convenience: async.await(awaitable)
    // Supports `wait()` or calling userdata directly (same semantics as time.timeout uses).
    t.set(
        "await",
        lua.create_async_function(|_lua, awaitable: AnyUserData| async move {
            if let Ok(wait_fn) = awaitable.get::<Function>("wait") {
                return wait_fn.call_async::<MultiValue>((awaitable.clone(),)).await;
            }

            awaitable
                .call_async::<MultiValue>(())
                .await
                .map_err(|_| mlua::Error::external("awaitable has neither wait() nor __call()"))
        })?,
    )?;

    Ok(t)
}
