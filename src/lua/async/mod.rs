#![allow(clippy::unnecessary_wraps)]

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, poll_fn};
use mlua::{
    AnyUserData, Function, Lua, MetaMethod, MultiValue, ObjectLike, RegistryKey, Table, UserData, UserDataMethods,
    Value, Variadic,
};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::Poll;
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot};

/// A Lua registry value with RAII cleanup.
///
/// This is critical for cancellation safety and to avoid leaks:
/// - If a message sits in a channel and the channel is dropped, pending messages will be dropped too.
/// - If a sender fails (closed/full), the rejected message is dropped.
///
/// In all those cases we must remove the registry entry.
#[derive(Debug)]
struct RegVal {
    lua: Lua,
    key: Option<RegistryKey>,
}

impl RegVal {
    fn new(lua: &Lua, v: Value) -> mlua::Result<Self> {
        Ok(Self {
            lua: lua.clone(),
            key: Some(lua.create_registry_value(v)?),
        })
    }

    fn into_value(mut self) -> mlua::Result<Value> {
        let Some(key) = self.key.take() else {
            return Ok(Value::Nil);
        };
        let v: Value = self.lua.registry_value(&key)?;
        self.lua.remove_registry_value(key)?;
        Ok(v)
    }
}

impl Drop for RegVal {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            let _ = self.lua.remove_registry_value(key);
        }
    }
}

async fn await_userdata(ud: AnyUserData) -> mlua::Result<MultiValue> {
    if let Ok(wait_fn) = ud.get::<Function>("wait") {
        return wait_fn.call_async::<MultiValue>((ud.clone(),)).await;
    }

    // Optional: calling userdata directly (requires `MetaMethod::Call`).
    match ud.call_async::<MultiValue>(()).await {
        Ok(v) => Ok(v),
        Err(e) => {
            // If the userdata isn't callable, Lua reports "attempt to call ...".
            // Preserve *real* call errors from a valid __call implementation.
            if let mlua::Error::RuntimeError(msg) = &e
                && msg.contains("attempt to call")
            {
                return Err(mlua::Error::external("awaitable must implement wait() (or __call())"));
            }
            Err(e)
        }
    }
}

async fn task_join(lua: Lua, this: &mut Task) -> mlua::Result<MultiValue> {
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
}

async fn channel_recv(lua: Lua, inner: Arc<ChannelInner>) -> mlua::Result<MultiValue> {
    // Serialize recv() across multiple consumers via async mutex.
    let mut rx = inner.rx.lock().await;
    let msg = rx.recv().await;
    drop(rx);

    match msg {
        Some(rv) => Ok(mv1(&lua, rv.into_value()?)),
        None => Ok(mv2(&lua, Value::Nil, Value::String(lua.create_string("closed")?))),
    }
}

#[derive(Debug)]
struct Task {
    rx: Option<oneshot::Receiver<mlua::Result<RegistryKey>>>,
    handle: Option<tokio::task::JoinHandle<()>>,
    abort_on_drop: bool,
}

impl Drop for Task {
    fn drop(&mut self) {
        // Structured concurrency default: if user drops the handle,
        // the task should not outlive the script.
        if let Some(h) = self.handle.take()
            && self.abort_on_drop
        {
            h.abort();
        }
        // Otherwise, dropping the JoinHandle detaches the task.
    }
}

impl UserData for Task {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("done", |_, this, ()| {
            Ok(this.handle.as_ref().is_none_or(tokio::task::JoinHandle::is_finished))
        });

        methods.add_method_mut("cancel", |_, this, ()| {
            let Some(h) = this.handle.take() else {
                return Ok(false);
            };
            h.abort();
            Ok(true)
        });

        methods.add_async_method_mut("wait", |lua, mut this, ()| async move { task_join(lua, &mut this).await });
        methods.add_method_mut("detach", |_, this, ()| {
            this.abort_on_drop = false;
            Ok(true)
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("Task()".to_string()));
    }
}

#[derive(Debug)]
struct ChannelInner {
    // tx is accessed from sync methods; keep std::sync::Mutex.
    tx: StdMutex<Option<mpsc::Sender<RegVal>>>,
    // rx must support concurrent recv() calls; serialize them with an async mutex.
    rx: TokioMutex<mpsc::Receiver<RegVal>>,
}

#[derive(Debug)]
struct Channel {
    inner: Arc<ChannelInner>,
}

impl Drop for Channel {
    fn drop(&mut self) {
        if let Ok(mut txg) = self.inner.tx.lock() {
            *txg = None;
        }
    }
}

impl UserData for Channel {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("send", |lua, this, v: Value| {
            let tx_res = this
                .inner
                .tx
                .lock()
                .map(|g| g.clone())
                .map_err(|_| mlua::Error::external("channel mutex poisoned"));

            async move {
                let closed = Value::String(lua.create_string("closed")?);
                let tx = tx_res?;

                let Some(tx) = tx else {
                    return Ok(mv2(&lua, Value::Nil, closed));
                };

                let permit = tx
                    .reserve()
                    .await
                    .map_err(|_| mlua::Error::RuntimeError("channel is closed".into()))?;
                let rv = RegVal::new(&lua, v)?;
                permit.send(rv);

                Ok(mv1(&lua, Value::Boolean(true)))
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

            let rv = RegVal::new(lua, v)?;
            match tx.clone().try_send(rv) {
                Ok(()) => Ok(mv1(lua, Value::Boolean(true))),
                Err(mpsc::error::TrySendError::Full(_k)) => {
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("full")?)))
                }
                Err(mpsc::error::TrySendError::Closed(_k)) => {
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("closed")?)))
                }
            }
        });

        methods.add_async_method("wait", |lua, this, ()| {
            let inner = this.inner.clone();
            async move { channel_recv(lua, inner).await }
        });

        methods.add_method("try_recv", |lua, this, ()| {
            let Ok(mut rx) = this.inner.rx.try_lock() else {
                // Someone is currently awaiting wait(); report as "busy".
                return Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("busy")?)));
            };
            match rx.try_recv() {
                Ok(rv) => Ok(mv1(lua, rv.into_value()?)),
                Err(mpsc::error::TryRecvError::Empty) => {
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("empty")?)))
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    Ok(mv2(lua, Value::Nil, Value::String(lua.create_string("closed")?)))
                }
            }
        });

        methods.add_method_mut("close", |_, this, ()| {
            let mut txg = this
                .inner
                .tx
                .lock()
                .map_err(|_| mlua::Error::external("channel mutex poisoned"))?;
            *txg = None;
            drop(txg);
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
            if n.fract() != 0.0 {
                return Err(mlua::Error::external("capacity must be an integer"));
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

            let mw_keys = crate::lua::process::proc_middleware_snapshot(lua)?;

            let handle = tokio::task::spawn_local(crate::lua::process::proc_middleware_scope(
                lua2.clone(),
                mw_keys,
                async move {
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
                        let len = i64::try_from(args_table.raw_len())
                            .map_err(|_| mlua::Error::external("too many arguments"))?;
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
                },
            ));

            lua.create_userdata(Task {
                rx: Some(rx),
                handle: Some(handle),
                abort_on_drop: true,
            })
        })?,
    )?;

    // async.channel(opts) -> Channel
    t.set(
        "channel",
        lua.create_function(|lua, opts: Value| {
            let cap = parse_capacity(opts)?;
            let (tx, rx) = mpsc::channel::<RegVal>(cap);
            lua.create_userdata(Channel {
                inner: Arc::new(ChannelInner {
                    tx: StdMutex::new(Some(tx)),
                    rx: TokioMutex::new(rx),
                }),
            })
        })?,
    )?;

    // Convenience: async.await(awaitable)
    // Contract: awaitable is a userdata with `wait()` (preferred) or `__call()`.
    t.set(
        "await",
        lua.create_async_function(|_lua, awaitable: AnyUserData| async move { await_userdata(awaitable).await })?,
    )?;

    // The input is an array-like table of userdatas.
    // Contract: each entry must implement `wait()` (preferred) or `__call()`.
    t.set(
        "select",
        lua.create_async_function(|_, list: Table| async move {
            let len_i64 = i64::try_from(list.raw_len()).map_err(|_| mlua::Error::external("too many awaitables"))?;
            if len_i64 <= 0 {
                return Err(mlua::Error::external("select expects a non-empty array table"));
            }
            let len = usize::try_from(len_i64).map_err(|_| mlua::Error::external("too many awaitables"))?;

            // Build per-entry futures in the *current* task and poll them in list order.
            // This avoids aborting a side-effecting await (e.g. Channel:recv) after it has
            // already consumed input but before results are delivered to Lua.
            let mut futs: Vec<BoxFuture<'static, mlua::Result<MultiValue>>> = Vec::with_capacity(len);

            for i in 1..=len_i64 {
                let v: Value = list.raw_get(i)?;
                let Value::UserData(ud) = v else {
                    return Err(mlua::Error::external("select expects an array of userdata awaitables"));
                };

                // Each future awaits one awaitable and yields its MultiValue.
                futs.push(async move { await_userdata(ud).await }.boxed());
            }

            // Biased selection: lowest index wins if multiple are ready in the same poll.
            let (idx, res) = poll_fn(|cx| {
                for (i, f) in futs.iter_mut().enumerate() {
                    match f.as_mut().poll(cx) {
                        Poll::Ready(r) => return Poll::Ready((i + 1, r)),
                        Poll::Pending => {}
                    }
                }
                Poll::Pending
            })
            .await;

            let out = res?;

            let mut mv = MultiValue::new();
            mv.push_back(Value::Integer(
                i64::try_from(idx).map_err(|_| mlua::Error::external("index overflow"))?,
            ));
            for v in out {
                mv.push_back(v);
            }
            Ok(mv)
        })?,
    )?;

    Ok(t)
}
