#![allow(clippy::missing_const_for_fn)]

use mlua::{
    Lua, MetaMethod, MultiValue, Result as LuaResult, Table, UserData, UserDataFields, UserDataMethods, Value, Variadic,
};
use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex as AsyncMutex,
    task::JoinHandle,
    time,
};

type EnvOverlay = HashMap<String, Option<String>>;
type BoxRead = Box<dyn AsyncRead + Unpin + Send + 'static>;

#[derive(Clone, Default)]
struct CmdSpec {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
    timeout_ms: Option<u64>,

    stdin: StdinSpec,
    stderr_to_stdout: bool,
}

#[derive(Clone, Default)]
enum StdinSpec {
    #[default]
    Inherit,
    Null,
    Bytes(Vec<u8>),
    File(PathBuf),
}

#[derive(Clone)]
struct Cmd {
    spec: CmdSpec,
}

#[derive(Clone)]
struct Pipeline {
    specs: Vec<CmdSpec>,
    pipefail: bool,
    timeout_ms: Option<u64>,
}

#[derive(Clone)]
struct CmdResult {
    ok: bool,
    code: i64,
    signal: Option<i64>,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
    steps: Vec<i64>,
}

type SharedReader = Arc<AsyncMutex<BufReader<BoxRead>>>;

#[derive(Clone)]
struct LineStream {
    inner: SharedReader,
}

#[derive(Clone)]
struct ByteStream {
    inner: SharedReader,
}

#[derive(Clone)]
struct ProcStdin {
    inner: Arc<AsyncMutex<Option<ChildStdin>>>,
}

#[derive(Clone)]
struct ProcChild {
    inner: Arc<AsyncMutex<ProcState>>,
    // PID snapshot at spawn time to avoid blocking the LocalSet thread.
    pids_snapshot: Arc<Vec<i64>>,
}

impl Drop for ProcChild {
    fn drop(&mut self) {
        // Best-effort cleanup: only act on the last clone and avoid blocking in Drop.
        if Arc::strong_count(&self.inner) != 1 {
            return;
        }

        if let Ok(mut st) = self.inner.try_lock() {
            st.abort_bg_tasks();
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let inner = self.inner.clone();
            handle.spawn(async move {
                let mut st = inner.lock().await;
                st.abort_bg_tasks();
            });
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamMode {
    Lines,
    Bytes,
}

struct ProcState {
    pipefail: bool,
    timeout_ms: Option<u64>,
    children: Vec<Child>,

    // Raw handles. These are kept until the user asks for a stream.
    stdin_raw: Option<ChildStdin>,
    stdout_raw: Option<BoxRead>,
    stderr_raw: Option<BoxRead>,
    stderr_merged: bool,

    // Once a stream is requested, we materialize a shared reader and remember which mode was chosen.
    stdout_reader: Option<SharedReader>,
    stderr_reader: Option<SharedReader>,
    stdout_mode: Option<StreamMode>,
    stderr_mode: Option<StreamMode>,

    // Background tasks:
    // - pipe pumps (stdout(i)->stdin(i+1), plus optional stderr pumps)
    // - optional stderr->stdout drain in inherit mode for last command
    // - optional stdin feeder for one-shot stdin
    link_tasks: Vec<JoinHandle<()>>,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
    aux_tasks: Vec<JoinHandle<()>>,
}

impl UserData for CmdResult {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("ok", |_, this| Ok(this.ok));
        fields.add_field_method_get("code", |_, this| Ok(this.code));
        fields.add_field_method_get("signal", |_, this| Ok(this.signal));
        fields.add_field_method_get("stdout", |lua, this| {
            this.stdout
                .as_ref()
                .map(|b| lua.create_string(b.as_slice()))
                .transpose()
        });
        fields.add_field_method_get("stderr", |lua, this| {
            this.stderr
                .as_ref()
                .map(|b| lua.create_string(b.as_slice()))
                .transpose()
        });
        fields.add_field_method_get("steps", |lua, this| {
            let t = lua.create_table()?;
            for (i, c) in this.steps.iter().enumerate() {
                t.set(i + 1, *c)?;
            }
            Ok(t)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_ok", |_, this, ()| Ok(this.ok));
        methods.add_method("assert_ok", |_, this, msg: Option<String>| {
            if this.ok {
                return Ok(());
            }
            let m = msg.unwrap_or_else(|| "process failed".to_string());
            Err(mlua::Error::RuntimeError(format!(
                "{m} (code={}, signal={:?})",
                this.code, this.signal
            )))
        });
    }
}

impl UserData for LineStream {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // stream:wait() -> line | nil, err
        // err is "eof" when the stream ends.
        methods.add_async_method("wait", |lua, this, ()| {
            let inner = this.inner.clone();
            async move {
                let mut guard = inner.lock().await;
                let mut buf = Vec::<u8>::new();
                match guard.read_until(b'\n', &mut buf).await {
                    Ok(0) => {
                        let mut mv = MultiValue::new();
                        mv.push_back(Value::Nil);
                        mv.push_back(Value::String(lua.create_string("eof")?));
                        Ok(mv)
                    }
                    Ok(_) => {
                        while buf.ends_with(b"\n") || buf.ends_with(b"\r") {
                            buf.pop();
                        }
                        let mut mv = MultiValue::new();
                        mv.push_back(Value::String(lua.create_string(buf.as_slice())?));
                        Ok(mv)
                    }
                    Err(e) => {
                        let mut mv = MultiValue::new();
                        mv.push_back(Value::Nil);
                        mv.push_back(Value::String(lua.create_string(e.to_string())?));
                        Ok(mv)
                    }
                }
            }
        });

        // Allow awaitable syntax: stream() == stream:wait()
        methods.add_async_meta_method(MetaMethod::Call, |lua, this, ()| {
            let inner = this.inner.clone();
            async move {
                let mut guard = inner.lock().await;
                let mut buf = Vec::<u8>::new();
                match guard.read_until(b'\n', &mut buf).await {
                    Ok(0) => {
                        let mut mv = MultiValue::new();
                        mv.push_back(Value::Nil);
                        mv.push_back(Value::String(lua.create_string("eof")?));
                        Ok(mv)
                    }
                    Ok(_) => {
                        while buf.ends_with(b"\n") || buf.ends_with(b"\r") {
                            buf.pop();
                        }
                        let mut mv = MultiValue::new();
                        mv.push_back(Value::String(lua.create_string(buf.as_slice())?));
                        Ok(mv)
                    }
                    Err(e) => {
                        let mut mv = MultiValue::new();
                        mv.push_back(Value::Nil);
                        mv.push_back(Value::String(lua.create_string(e.to_string())?));
                        Ok(mv)
                    }
                }
            }
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("LineStream()".to_string()));
    }
}

impl UserData for ByteStream {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // bytes:read(n?) -> chunk | nil, err
        // - n defaults to 16384
        // - err is "eof" when the stream ends
        methods.add_async_method("read", |lua, this, n: Option<i64>| {
            let inner = this.inner.clone();
            async move { read_chunk(&lua, inner, n).await }
        });

        // Awaitable helper: bytes:wait(n?) == bytes:read(n?)
        methods.add_async_method("wait", |lua, this, n: Option<i64>| {
            let inner = this.inner.clone();
            async move { read_chunk(&lua, inner, n).await }
        });

        // Allow awaitable syntax: bytes(n?) == bytes:wait(n?)
        methods.add_async_meta_method(MetaMethod::Call, |lua, this, n: Option<i64>| {
            let inner = this.inner.clone();
            async move { read_chunk(&lua, inner, n).await }
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("ByteStream()".to_string()));
    }
}

async fn read_chunk(lua: &Lua, inner: SharedReader, n: Option<i64>) -> LuaResult<MultiValue> {
    let n = n.unwrap_or(16 * 1024);
    if n <= 0 {
        return Err(mlua::Error::RuntimeError("read(n): n must be > 0".into()));
    }
    let n = usize::try_from(n).map_err(|_| mlua::Error::RuntimeError("read(n): n is too large".into()))?;

    let mut guard = inner.lock().await;
    let mut buf = vec![0_u8; n];
    match guard.read(&mut buf).await {
        Ok(0) => {
            let mut mv = MultiValue::new();
            mv.push_back(Value::Nil);
            mv.push_back(Value::String(lua.create_string("eof")?));
            Ok(mv)
        }
        Ok(sz) => {
            buf.truncate(sz);
            let mut mv = MultiValue::new();
            mv.push_back(Value::String(lua.create_string(buf.as_slice())?));
            Ok(mv)
        }
        Err(e) => {
            let mut mv = MultiValue::new();
            mv.push_back(Value::Nil);
            mv.push_back(Value::String(lua.create_string(e.to_string())?));
            Ok(mv)
        }
    }
}

impl ProcStdin {
    fn new(w: ChildStdin) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(w))),
        }
    }
}

impl UserData for ProcStdin {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("write", |lua, this, bytes: Value| {
            let inner = this.inner.clone();
            async move {
                let Value::String(s) = bytes else {
                    return Err(mlua::Error::RuntimeError("stdin:write(bytes): expected string".into()));
                };

                let mut guard = inner.lock().await;
                let Some(w) = guard.as_mut() else {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("closed")?));
                    return Ok(mv);
                };

                if let Err(e) = w.write_all(s.as_bytes().as_ref()).await {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string(e.to_string())?));
                    return Ok(mv);
                }

                drop(guard);
                let mut mv = MultiValue::new();
                mv.push_back(Value::Boolean(true));
                Ok(mv)
            }
        });

        methods.add_async_method("writeln", |lua, this, s: String| {
            let inner = this.inner.clone();
            async move {
                let mut guard = inner.lock().await;
                let Some(w) = guard.as_mut() else {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("closed")?));
                    return Ok(mv);
                };

                if let Err(e) = w.write_all(s.as_bytes().as_ref()).await {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string(e.to_string())?));
                    return Ok(mv);
                }
                if let Err(e) = w.write_all(b"\n").await {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string(e.to_string())?));
                    return Ok(mv);
                }

                drop(guard);
                let mut mv = MultiValue::new();
                mv.push_back(Value::Boolean(true));
                Ok(mv)
            }
        });

        methods.add_async_method("flush", |lua, this, ()| {
            let inner = this.inner.clone();
            async move {
                let mut guard = inner.lock().await;
                let Some(w) = guard.as_mut() else {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("closed")?));
                    return Ok(mv);
                };

                if let Err(e) = w.flush().await {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string(e.to_string())?));
                    return Ok(mv);
                }

                drop(guard);
                let mut mv = MultiValue::new();
                mv.push_back(Value::Boolean(true));
                Ok(mv)
            }
        });

        methods.add_async_method("close", |_, this, ()| {
            let inner = this.inner.clone();
            async move {
                inner.lock().await.take();
                Ok(true)
            }
        });

        // NOTE: This is intentionally non-blocking.
        // Using `blocking_lock()` here can deadlock if a write/flush is in progress,
        // because those methods hold the same mutex across an async I/O await.
        methods.add_method("is_closed", |_, this, ()| {
            Ok(this.inner.try_lock().is_ok_and(|g| g.is_none()))
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("ProcStdin()".to_string()));
    }
}

impl Cmd {
    fn new(program: String, args: Vec<String>) -> Self {
        Self {
            spec: CmdSpec {
                program,
                args,
                ..Default::default()
            },
        }
    }

    fn snapshot(&self) -> CmdSpec {
        self.spec.clone()
    }
}

impl UserData for Cmd {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cwd", |_, this, path: String| {
            let mut spec = this.snapshot();
            spec.cwd = Some(PathBuf::from(path));
            Ok(Self { spec })
        });

        methods.add_method("env", |_, this, (k, v): (String, String)| {
            let mut spec = this.snapshot();
            spec.env.insert(k, v);
            Ok(Self { spec })
        });

        methods.add_method("envs", |_, this, t: Table| {
            let mut spec = this.snapshot();
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                if let (Value::String(ks), Value::String(vs)) = (k, v) {
                    spec.env.insert(ks.to_str()?.to_string(), vs.to_str()?.to_string());
                }
            }
            Ok(Self { spec })
        });

        #[allow(clippy::cast_sign_loss)]
        methods.add_method("timeout", |_, this, ms: i64| {
            let mut spec = this.snapshot();
            spec.timeout_ms = Some(ms.max(0) as u64);
            Ok(Self { spec })
        });

        methods.add_method("stdin", |_, this, v: Value| {
            let mut spec = this.snapshot();
            spec.stdin = match v {
                Value::Nil | Value::Boolean(false) => StdinSpec::Inherit,
                Value::String(s) => StdinSpec::Bytes(s.as_bytes().to_vec()),
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "stdin(v): expected bytes string (or nil/false to reset), got {}",
                        other.type_name()
                    )));
                }
            };
            Ok(Self { spec })
        });

        methods.add_method("stdin_file", |_, this, path: String| {
            let mut spec = this.snapshot();
            spec.stdin = StdinSpec::File(PathBuf::from(path));
            Ok(Self { spec })
        });

        methods.add_method("stdin_null", |_, this, ()| {
            let mut spec = this.snapshot();
            spec.stdin = StdinSpec::Null;
            Ok(Self { spec })
        });

        methods.add_method("stderr_to_stdout", |_, this, yes: Option<bool>| {
            let mut spec = this.snapshot();
            spec.stderr_to_stdout = yes.unwrap_or(true);
            Ok(Self { spec })
        });

        methods.add_method("pipe", |_, this, rhs: Value| pipe_value(this.snapshot(), rhs));

        methods.add_async_method("run", |lua, this, ()| {
            let spec = this.snapshot();
            async move {
                let overlay = crate::lua::env::overlay_snapshot(&lua)?;
                run_specs(vec![spec], false, RunMode::Inherit, overlay).await
            }
        });

        methods.add_async_method("output", |lua, this, ()| {
            let spec = this.snapshot();
            async move {
                let overlay = crate::lua::env::overlay_snapshot(&lua)?;
                run_specs(vec![spec], false, RunMode::Capture, overlay).await
            }
        });

        // cmd:spawn(opts?) -> ProcChild
        methods.add_async_method("spawn", |lua, this, opts: Option<Table>| {
            let spec = this.snapshot();
            async move {
                let overlay = crate::lua::env::overlay_snapshot(&lua)?;
                let cfg = SpawnCfg::from_lua(opts.as_ref(), &spec)?;
                spawn_specs(vec![spec], false, None, cfg, overlay).await
            }
        });

        methods.add_meta_method(MetaMethod::BOr, |_, this, rhs: Value| pipe_value(this.snapshot(), rhs));

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("Cmd()".to_string()));
    }
}

impl Pipeline {
    fn new(specs: Vec<CmdSpec>, pipefail: bool) -> Self {
        let timeout_ms = None;
        Self {
            specs,
            pipefail,
            timeout_ms,
        }
    }

    fn snapshot(&self) -> (Vec<CmdSpec>, bool, Option<u64>) {
        (self.specs.clone(), self.pipefail, self.timeout_ms)
    }
}

impl UserData for Pipeline {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("pipefail", |_, this, yes: Option<bool>| {
            let (specs, pipefail, timeout_ms) = this.snapshot();
            Ok(Self {
                specs,
                pipefail: yes.unwrap_or(true) || pipefail,
                timeout_ms,
            })
        });

        #[allow(clippy::cast_sign_loss)]
        methods.add_method("timeout", |_, this, ms: i64| {
            let (specs, pipefail, _) = this.snapshot();
            Ok(Self {
                specs,
                pipefail,
                timeout_ms: Some(ms.max(0) as u64),
            })
        });

        methods.add_method("pipe", |_, this, rhs: Value| {
            let (mut specs, pipefail, timeout_ms) = this.snapshot();
            match rhs {
                Value::UserData(ud) => {
                    if let Ok(cmd) = ud.borrow::<Cmd>() {
                        specs.push(cmd.snapshot());
                        Ok(Self {
                            specs,
                            pipefail,
                            timeout_ms,
                        })
                    } else if let Ok(p) = ud.borrow::<Self>() {
                        let (p_specs, p_pipefail, p_timeout) = p.snapshot();
                        specs.extend(p_specs);
                        Ok(Self {
                            specs,
                            pipefail: pipefail || p_pipefail,
                            timeout_ms: timeout_ms.or(p_timeout),
                        })
                    } else {
                        Err(mlua::Error::RuntimeError("pipe(): expected Cmd or Pipeline".into()))
                    }
                }
                _ => Err(mlua::Error::RuntimeError("pipe(): expected Cmd or Pipeline".into())),
            }
        });

        methods.add_async_method("run", |lua, this, ()| {
            let (specs, pipefail, timeout_ms) = this.snapshot();
            async move {
                let overlay = crate::lua::env::overlay_snapshot(&lua)?;
                run_specs(specs, pipefail, RunMode::InheritWithTimeout(timeout_ms), overlay).await
            }
        });

        methods.add_async_method("output", |lua, this, ()| {
            let (specs, pipefail, timeout_ms) = this.snapshot();
            async move {
                let overlay = crate::lua::env::overlay_snapshot(&lua)?;
                run_specs(specs, pipefail, RunMode::CaptureWithTimeout(timeout_ms), overlay).await
            }
        });

        methods.add_async_method("spawn", |lua, this, opts: Option<Table>| {
            let (specs, pipefail, timeout_ms) = this.snapshot();
            async move {
                let overlay = crate::lua::env::overlay_snapshot(&lua)?;
                let cfg = SpawnCfg::from_lua(opts.as_ref(), &specs.last().cloned().unwrap_or_else(CmdSpec::default))?;
                spawn_specs(specs, pipefail, timeout_ms, cfg, overlay).await
            }
        });

        methods.add_meta_method(MetaMethod::BOr, |_, this, rhs: Value| {
            let (mut specs, pipefail, timeout_ms) = this.snapshot();
            match rhs {
                Value::UserData(ud) => {
                    if let Ok(cmd) = ud.borrow::<Cmd>() {
                        specs.push(cmd.snapshot());
                        Ok(Self {
                            specs,
                            pipefail,
                            timeout_ms,
                        })
                    } else if let Ok(p) = ud.borrow::<Self>() {
                        let (p_specs, p_pipefail, p_timeout) = p.snapshot();
                        specs.extend(p_specs);
                        Ok(Self {
                            specs,
                            pipefail: pipefail || p_pipefail,
                            timeout_ms: timeout_ms.or(p_timeout),
                        })
                    } else {
                        Err(mlua::Error::RuntimeError("operator | expects Cmd or Pipeline".into()))
                    }
                }
                _ => Err(mlua::Error::RuntimeError("operator | expects Cmd or Pipeline".into())),
            }
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("Pipeline()".to_string()));
    }
}

impl UserData for ProcChild {
    #[allow(clippy::too_many_lines)]
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("pid", |_, this, ()| {
            // Primary pid is the last stage.
            Ok(this.pids_snapshot.last().copied().unwrap_or(0))
        });

        methods.add_method("pids", |lua, this, ()| {
            let t = lua.create_table()?;
            for (i, pid) in this.pids_snapshot.iter().enumerate() {
                t.set(i + 1, *pid)?;
            }
            Ok(t)
        });

        methods.add_async_method("stdin", |lua, this, ()| {
            let inner = this.inner.clone();
            async move {
                let mut st = inner.lock().await;
                let Some(w) = st.stdin_raw.take() else {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("not_piped")?));
                    return Ok(mv);
                };
                drop(st);
                let mut mv = MultiValue::new();
                mv.push_back(Value::UserData(lua.create_userdata(ProcStdin::new(w))?));
                Ok(mv)
            }
        });

        methods.add_async_method("stdout_lines", |lua, this, ()| {
            let inner = this.inner.clone();
            async move { get_stream(&lua, inner, StreamWhich::Stdout, StreamMode::Lines).await }
        });

        methods.add_async_method("stderr_lines", |lua, this, ()| {
            let inner = this.inner.clone();
            async move { get_stream(&lua, inner, StreamWhich::Stderr, StreamMode::Lines).await }
        });

        methods.add_async_method("stdout_bytes", |lua, this, ()| {
            let inner = this.inner.clone();
            async move { get_stream(&lua, inner, StreamWhich::Stdout, StreamMode::Bytes).await }
        });

        methods.add_async_method("stderr_bytes", |lua, this, ()| {
            let inner = this.inner.clone();
            async move { get_stream(&lua, inner, StreamWhich::Stderr, StreamMode::Bytes).await }
        });

        methods.add_async_method("kill", |_, this, ()| {
            let inner = this.inner.clone();
            async move {
                let mut st = inner.lock().await;
                let mut killed_any = false;
                for ch in &mut st.children {
                    if ch.id().is_some() && ch.kill().await.is_ok() {
                        killed_any = true;
                    }
                }
                drop(st);
                Ok(killed_any)
            }
        });

        methods.add_async_method("wait", |_, this, ()| {
            let inner = this.inner.clone();
            async move {
                let mut st = inner.lock().await;

                // Drain any piped stdio that wasn't turned into a Lua stream.
                st.drain_unclaimed_stdio();

                let timeout_ms = st.timeout_ms;
                let outcome = wait_children_with_timeout(&mut st.children, timeout_ms).await?;

                if outcome.timed_out {
                    // On timeout we abort pumps/aux to avoid hangs on broken pipes.
                    for t in &st.link_tasks {
                        t.abort();
                    }
                    for t in &st.aux_tasks {
                        t.abort();
                    }
                }

                // Ensure pipe tasks have finished.
                for t in st.link_tasks.drain(..) {
                    let _ = t.await;
                }

                if let Some(t) = st.stdin_task.take() {
                    match t.await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => return Err(mlua::Error::external(e)),
                        Err(_) => return Err(mlua::Error::RuntimeError("stdin feeder task panicked".into())),
                    }
                }

                for t in st.aux_tasks.drain(..) {
                    let _ = t.await;
                }

                let ok = if st.pipefail {
                    outcome.steps.iter().all(|c| *c == 0)
                } else {
                    outcome.code == 0
                };
                drop(st);
                Ok(CmdResult {
                    ok,
                    code: outcome.code,
                    signal: outcome.signal,
                    stdout: None,
                    stderr: None,
                    steps: outcome.steps,
                })
            }
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("ProcChild()".to_string()));
    }
}

impl ProcState {
    fn drain_unclaimed_stdio(&mut self) {
        if self.stdout_reader.is_none()
            && let Some(r) = self.stdout_raw.take()
        {
            self.aux_tasks.push(tokio::spawn(async move {
                let mut rr = r;
                let mut sink = io::sink();
                let _ = io::copy(&mut rr, &mut sink).await;
            }));
        }

        if self.stderr_reader.is_none()
            && let Some(r) = self.stderr_raw.take()
        {
            self.aux_tasks.push(tokio::spawn(async move {
                let mut rr = r;
                let mut sink = io::sink();
                let _ = io::copy(&mut rr, &mut sink).await;
            }));
        }
    }

    fn abort_bg_tasks(&mut self) {
        for t in self.link_tasks.drain(..) {
            t.abort();
        }
        if let Some(t) = self.stdin_task.take() {
            t.abort();
        }
        for t in self.aux_tasks.drain(..) {
            t.abort();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamWhich {
    Stdout,
    Stderr,
}

async fn get_stream(
    lua: &Lua,
    inner: Arc<AsyncMutex<ProcState>>,
    which: StreamWhich,
    mode: StreamMode,
) -> LuaResult<MultiValue> {
    let mut st = inner.lock().await;

    if which == StreamWhich::Stderr && st.stderr_merged {
        let mut mv = MultiValue::new();
        mv.push_back(Value::Nil);
        mv.push_back(Value::String(lua.create_string("merged")?));
        return Ok(mv);
    }

    // Avoid borrowing `st` mutably more than once across control flow by fully
    // separating the stdout/stderr paths.
    match which {
        StreamWhich::Stdout => {
            if let Some(cur) = st.stdout_mode {
                if cur != mode {
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("mode_conflict")?));
                    return Ok(mv);
                }
            } else {
                st.stdout_mode = Some(mode);
            }

            let shared = if let Some(r) = st.stdout_reader.as_ref() {
                r.clone()
            } else {
                let Some(raw) = st.stdout_raw.take() else {
                    drop(st);
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("not_piped")?));
                    return Ok(mv);
                };
                let shared: SharedReader = Arc::new(AsyncMutex::new(BufReader::new(raw)));
                st.stdout_reader = Some(shared.clone());
                shared
            };

            drop(st);
            let mut mv = MultiValue::new();
            match mode {
                StreamMode::Lines => mv.push_back(Value::UserData(lua.create_userdata(LineStream { inner: shared })?)),
                StreamMode::Bytes => mv.push_back(Value::UserData(lua.create_userdata(ByteStream { inner: shared })?)),
            }
            Ok(mv)
        }
        StreamWhich::Stderr => {
            if let Some(cur) = st.stderr_mode {
                if cur != mode {
                    drop(st);
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("mode_conflict")?));
                    return Ok(mv);
                }
            } else {
                st.stderr_mode = Some(mode);
            }

            let shared = if let Some(r) = st.stderr_reader.as_ref() {
                r.clone()
            } else {
                let Some(raw) = st.stderr_raw.take() else {
                    drop(st);
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::Nil);
                    mv.push_back(Value::String(lua.create_string("not_piped")?));
                    return Ok(mv);
                };
                let shared: SharedReader = Arc::new(AsyncMutex::new(BufReader::new(raw)));
                st.stderr_reader = Some(shared.clone());
                shared
            };

            drop(st);
            let mut mv = MultiValue::new();
            match mode {
                StreamMode::Lines => mv.push_back(Value::UserData(lua.create_userdata(LineStream { inner: shared })?)),
                StreamMode::Bytes => mv.push_back(Value::UserData(lua.create_userdata(ByteStream { inner: shared })?)),
            }
            Ok(mv)
        }
    }
}

#[derive(Clone, Copy)]
enum RunMode {
    Inherit,
    Capture,
    InheritWithTimeout(Option<u64>),
    CaptureWithTimeout(Option<u64>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StdioMode {
    Inherit,
    Pipe,
    Null,
}

#[derive(Clone, Copy)]
struct SpawnCfg {
    stdin: StdioMode,
    stdout: StdioMode,
    stderr: StdioMode,
}

impl SpawnCfg {
    fn from_lua(opts: Option<&Table>, spec: &CmdSpec) -> LuaResult<Self> {
        let mut cfg = Self {
            stdin: StdioMode::Inherit,
            // Spawn is usually used for streaming.
            stdout: StdioMode::Pipe,
            stderr: if spec.stderr_to_stdout {
                StdioMode::Pipe
            } else {
                StdioMode::Inherit
            },
        };

        let Some(t) = opts else {
            return Ok(cfg);
        };
        cfg.stdin = parse_stdio_mode(t.get::<Option<Value>>("stdin")?, cfg.stdin)?;
        cfg.stdout = parse_stdio_mode(t.get::<Option<Value>>("stdout")?, cfg.stdout)?;
        cfg.stderr = parse_stdio_mode(t.get::<Option<Value>>("stderr")?, cfg.stderr)?;
        Ok(cfg)
    }
}

fn parse_stdio_mode(v: Option<Value>, default: StdioMode) -> LuaResult<StdioMode> {
    let Some(v) = v else {
        return Ok(default);
    };
    match v {
        Value::Nil => Ok(default),
        Value::Boolean(b) => Ok(if b { StdioMode::Pipe } else { StdioMode::Inherit }),
        Value::String(s) => {
            let s = s.to_str()?.to_lowercase();
            match s.as_str() {
                "pipe" => Ok(StdioMode::Pipe),
                "inherit" => Ok(StdioMode::Inherit),
                "null" => Ok(StdioMode::Null),
                _ => Err(mlua::Error::RuntimeError(
                    "stdio must be true/false or 'pipe'/'inherit'/'null'".into(),
                )),
            }
        }
        _ => Err(mlua::Error::RuntimeError(
            "stdio must be true/false or 'pipe'/'inherit'/'null'".into(),
        )),
    }
}

fn apply_env_overlay(cmd: &mut Command, overlay: &EnvOverlay) {
    for (k, v) in overlay {
        match v {
            Some(val) => {
                cmd.env(k, val);
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
}

fn apply_common_opts(cmd: &mut Command, spec: &CmdSpec) {
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    if !spec.env.is_empty() {
        cmd.envs(spec.env.iter());
    }
}

struct SpawnedCore {
    pipefail: bool,
    timeout_ms: Option<u64>,
    children: Vec<Child>,
    pids: Vec<i64>,
    stdin: Option<ChildStdin>,
    stdout: Option<BoxRead>,
    stderr: Option<BoxRead>,
    stderr_merged: bool,
    link_tasks: Vec<JoinHandle<()>>,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
    aux_tasks: Vec<JoinHandle<()>>,
}

#[allow(clippy::too_many_lines)]
async fn run_specs(specs: Vec<CmdSpec>, pipefail: bool, mode: RunMode, overlay: EnvOverlay) -> LuaResult<CmdResult> {
    if specs.is_empty() {
        return Err(mlua::Error::RuntimeError("empty pipeline".into()));
    }

    let (capture, timeout_override) = match mode {
        RunMode::Inherit => (false, None),
        RunMode::Capture => (true, None),
        RunMode::InheritWithTimeout(t) => (false, t),
        RunMode::CaptureWithTimeout(t) => (true, t),
    };

    let cfg = if capture {
        SpawnCfg {
            stdin: StdioMode::Inherit,
            stdout: StdioMode::Pipe,
            stderr: StdioMode::Pipe,
        }
    } else {
        SpawnCfg {
            stdin: StdioMode::Inherit,
            stdout: StdioMode::Inherit,
            stderr: StdioMode::Inherit,
        }
    };

    let core = spawn_specs_core(specs.clone(), pipefail, timeout_override, cfg, overlay).await?;
    let SpawnedCore {
        timeout_ms,
        mut children,
        stdout,
        stderr,
        link_tasks,
        stdin_task,
        aux_tasks,
        ..
    } = core;

    let mut stdout_task: Option<JoinHandle<Vec<u8>>> = None;
    let mut stderr_task: Option<JoinHandle<Vec<u8>>> = None;

    if capture {
        if let Some(out) = stdout {
            stdout_task = Some(tokio::spawn(async move {
                let mut r = out;
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf).await;
                buf
            }));
        }
        if let Some(err) = stderr {
            stderr_task = Some(tokio::spawn(async move {
                let mut r = err;
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf).await;
                buf
            }));
        }
    }

    let outcome = wait_children_with_timeout(&mut children, timeout_ms).await?;

    if outcome.timed_out {
        for t in &link_tasks {
            t.abort();
        }
        if let Some(t) = &stdin_task {
            t.abort();
        }
        for t in &aux_tasks {
            t.abort();
        }
    }

    for t in link_tasks {
        let _ = t.await;
    }

    if let Some(t) = stdin_task {
        match t.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(mlua::Error::external(e)),
            Err(_) => return Err(mlua::Error::RuntimeError("stdin feeder task panicked".into())),
        }
    }

    for t in aux_tasks {
        let _ = t.await;
    }

    let stdout = match stdout_task {
        Some(t) => t.await.ok(),
        None => None,
    };
    let stderr = match stderr_task {
        Some(t) => t.await.ok(),
        None => None,
    };

    let ok = if pipefail {
        outcome.steps.iter().all(|c| *c == 0)
    } else {
        outcome.code == 0
    };

    Ok(CmdResult {
        ok,
        code: outcome.code,
        signal: outcome.signal,
        stdout,
        stderr,
        steps: outcome.steps,
    })
}

async fn spawn_specs(
    specs: Vec<CmdSpec>,
    pipefail: bool,
    timeout_ms: Option<u64>,
    cfg: SpawnCfg,
    overlay: EnvOverlay,
) -> LuaResult<ProcChild> {
    if specs.is_empty() {
        return Err(mlua::Error::RuntimeError("empty pipeline".into()));
    }
    let core = spawn_specs_core(specs, pipefail, timeout_ms, cfg, overlay).await?;
    let pids_snapshot = Arc::new(core.pids.clone());
    Ok(ProcChild {
        inner: Arc::new(AsyncMutex::new(ProcState {
            pipefail: core.pipefail,
            timeout_ms: core.timeout_ms,
            children: core.children,
            stdin_raw: core.stdin,
            stdout_raw: core.stdout,
            stderr_raw: core.stderr,
            stderr_merged: core.stderr_merged,
            stdout_reader: None,
            stderr_reader: None,
            stdout_mode: None,
            stderr_mode: None,
            link_tasks: core.link_tasks,
            stdin_task: core.stdin_task,
            aux_tasks: core.aux_tasks,
        })),
        pids_snapshot,
    })
}

#[allow(clippy::too_many_lines, clippy::unused_async)]
async fn spawn_specs_core(
    mut specs: Vec<CmdSpec>,
    pipefail: bool,
    timeout_override: Option<u64>,
    cfg: SpawnCfg,
    overlay: EnvOverlay,
) -> LuaResult<SpawnedCore> {
    if specs.is_empty() {
        return Err(mlua::Error::RuntimeError("empty pipeline".into()));
    }

    // Pipeline-level timeout: explicit override wins, otherwise use the smallest non-zero timeout among steps.
    let timeout_ms = timeout_override.or_else(|| min_nonzero_timeout(&specs));

    let n = specs.len();
    let last = n - 1;

    // Validate stdin feeding vs interactive piping.
    let first_stdin_preset = matches!(specs[0].stdin, StdinSpec::Bytes(_) | StdinSpec::File(_));
    if first_stdin_preset && cfg.stdin == StdioMode::Pipe {
        return Err(mlua::Error::RuntimeError(
            "spawn(): cannot combine cmd:stdin(...) / cmd:stdin_file(...) with spawn({ stdin = 'pipe' })".into(),
        ));
    }

    // Pre-open stdin_file so errors surface at spawn-time and we can avoid a feeder task.
    let mut preset_stdin = if let StdinSpec::File(path) = &specs[0].stdin {
        let f = File::open(path).map_err(mlua::Error::external)?;
        Some(std::process::Stdio::from(f))
    } else {
        None
    };

    // Spawn all children.
    let mut children: Vec<Child> = Vec::with_capacity(n);
    let mut pids: Vec<i64> = Vec::with_capacity(n);

    for (i, spec) in specs.iter_mut().enumerate() {
        let mut c = Command::new(&spec.program);
        c.kill_on_drop(true);
        c.args(&spec.args);
        apply_env_overlay(&mut c, &overlay);
        apply_common_opts(&mut c, spec);

        // stdin
        if i == 0 {
            if let Some(stdio) = preset_stdin.take() {
                c.stdin(stdio);
            } else {
                match (&spec.stdin, cfg.stdin) {
                    (StdinSpec::Bytes(_), _) | (StdinSpec::Inherit, StdioMode::Pipe) => {
                        c.stdin(std::process::Stdio::piped());
                    }
                    (StdinSpec::Null, _) | (StdinSpec::Inherit, StdioMode::Null) => {
                        c.stdin(std::process::Stdio::null());
                    }
                    (StdinSpec::Inherit, StdioMode::Inherit) => {
                        c.stdin(std::process::Stdio::inherit());
                    }
                    (StdinSpec::File(_), _) => {
                        return Err(mlua::Error::RuntimeError(
                            "stdin_file: internal error (file was not pre-opened)".into(),
                        ));
                    }
                }
            }
        } else {
            c.stdin(std::process::Stdio::piped());
        }

        // stdout
        if i < last {
            c.stdout(std::process::Stdio::piped());
        } else {
            match cfg.stdout {
                StdioMode::Inherit => c.stdout(std::process::Stdio::inherit()),
                StdioMode::Pipe => c.stdout(std::process::Stdio::piped()),
                StdioMode::Null => c.stdout(std::process::Stdio::null()),
            };
        }

        // stderr
        if i < last {
            if spec.stderr_to_stdout {
                c.stderr(std::process::Stdio::piped());
            } else {
                c.stderr(std::process::Stdio::inherit());
            }
        } else if spec.stderr_to_stdout {
            // Must be piped so we can merge/passthrough.
            c.stderr(std::process::Stdio::piped());
        } else {
            match cfg.stderr {
                StdioMode::Inherit => c.stderr(std::process::Stdio::inherit()),
                StdioMode::Pipe => c.stderr(std::process::Stdio::piped()),
                StdioMode::Null => c.stderr(std::process::Stdio::null()),
            };
        }

        let ch = c.spawn().map_err(mlua::Error::external)?;
        pids.push(ch.id().map_or(0, i64::from));
        children.push(ch);
    }

    // Wire pipes.
    let mut link_tasks: Vec<JoinHandle<()>> = Vec::new();
    for i in 0..last {
        let out_i: ChildStdout = children[i]
            .stdout
            .take()
            .ok_or_else(|| mlua::Error::RuntimeError("missing stdout for pipe".into()))?;
        let in_next: ChildStdin = children[i + 1]
            .stdin
            .take()
            .ok_or_else(|| mlua::Error::RuntimeError("missing stdin for pipe".into()))?;

        let shared = Arc::new(AsyncMutex::new(in_next));

        // If this stage merges stderr into the pipeline, two concurrent pumps will feed the same
        // stdin (stdout and stderr). We must not close stdin until *both* pumps complete.
        let err_i = if specs[i].stderr_to_stdout {
            children[i].stderr.take()
        } else {
            None
        };
        let close_guard = Arc::new(AtomicUsize::new(1 + usize::from(err_i.is_some())));

        link_tasks.push(tokio::spawn(pump_to_shared_stdin(
            out_i,
            shared.clone(),
            close_guard.clone(),
        )));

        if let Some(err) = err_i {
            link_tasks.push(tokio::spawn(pump_to_shared_stdin(err, shared, close_guard)));
        }
    }

    // Final handles.
    let stdin_handle: Option<ChildStdin> = if cfg.stdin == StdioMode::Pipe {
        children[0].stdin.take()
    } else {
        None
    };

    // Background tasks.
    let mut aux_tasks: Vec<JoinHandle<()>> = Vec::new();

    // One-shot stdin feed (bytes only). stdin_file is wired directly to the child to surface open errors.
    let stdin_task: Option<JoinHandle<io::Result<()>>> = if matches!(specs[0].stdin, StdinSpec::Bytes(_)) {
        let Some(w) = children[0].stdin.take() else {
            return Err(mlua::Error::RuntimeError("missing stdin for configured input".into()));
        };
        let spec = specs[0].clone();
        Some(tokio::spawn(async move { feed_preset_stdin(w, spec.stdin).await }))
    } else {
        None
    };

    // Last-stage stdout/stderr handles.
    let mut stdout_handle: Option<BoxRead> = None;
    let mut stderr_handle: Option<BoxRead> = None;
    let mut stderr_merged = false;

    let last_spec = specs[last].clone();
    match cfg.stdout {
        StdioMode::Pipe => {
            stdout_handle = children[last].stdout.take().map(|x| Box::new(x) as BoxRead);
        }
        StdioMode::Inherit | StdioMode::Null => {}
    }

    // If last stage has stderr_to_stdout, treat stderr as "mergeable" regardless of cfg.stderr.
    if last_spec.stderr_to_stdout {
        let err = children[last].stderr.take().map(|x| Box::new(x) as BoxRead);
        if cfg.stdout == StdioMode::Pipe {
            // Merge into a single reader using a duplex.
            let Some(out) = stdout_handle.take() else {
                return Err(mlua::Error::RuntimeError(
                    "stderr_to_stdout requires piped stdout in this mode".into(),
                ));
            };
            let Some(err) = err else {
                return Err(mlua::Error::RuntimeError("missing stderr for merge".into()));
            };

            let (rx, tx) = io::duplex(16 * 1024);
            let writer = Arc::new(AsyncMutex::new(tx));
            aux_tasks.push(tokio::spawn(pump_to_duplex(out, writer.clone())));
            aux_tasks.push(tokio::spawn(pump_to_duplex(err, writer)));
            stdout_handle = Some(Box::new(rx) as BoxRead);
            stderr_handle = None;
            stderr_merged = true;
        } else {
            // Inherit mode: drain stderr and forward to parent stdout.
            if let Some(err) = err {
                aux_tasks.push(tokio::spawn(async move {
                    let mut r = err;
                    let mut out = tokio::io::stdout();
                    let mut buf = [0_u8; 16 * 1024];
                    loop {
                        match r.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let _ = out.write_all(&buf[..n]).await;
                                let _ = out.flush().await;
                            }
                        }
                    }
                }));
            }
            stderr_handle = None;
            stderr_merged = true;
        }
    } else if cfg.stderr == StdioMode::Pipe {
        stderr_handle = children[last].stderr.take().map(|x| Box::new(x) as BoxRead);
    }

    Ok(SpawnedCore {
        pipefail,
        timeout_ms,
        children,
        pids,
        stdin: stdin_handle,
        stdout: stdout_handle,
        stderr: stderr_handle,
        stderr_merged,
        link_tasks,
        stdin_task,
        aux_tasks,
    })
}

fn min_nonzero_timeout(specs: &[CmdSpec]) -> Option<u64> {
    let mut out: Option<u64> = None;
    for s in specs {
        if let Some(ms) = s.timeout_ms {
            if ms == 0 {
                continue;
            }
            out = out.map_or(Some(ms), |prev| Some(prev.min(ms)));
        }
    }
    out
}

async fn pump_to_shared_stdin<R>(
    mut reader: R,
    stdin: Arc<AsyncMutex<ChildStdin>>,
    close_guard: Arc<AtomicUsize>,
)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut w = stdin.lock().await;
                if w.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
    // Close stdin only after the last upstream pump finishes.
    // This preserves correct behavior for `stderr_to_stdout` in pipelines.
    if close_guard.fetch_sub(1, Ordering::AcqRel) == 1 {
        let _ = stdin.lock().await.shutdown().await;
    }
}

async fn pump_to_duplex(mut reader: BoxRead, writer: Arc<AsyncMutex<io::DuplexStream>>) {
    let mut buf = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut w = writer.lock().await;
                if w.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn feed_preset_stdin(mut w: ChildStdin, stdin: StdinSpec) -> io::Result<()> {
    match stdin {
        StdinSpec::Bytes(b) => {
            if let Err(e) = w.write_all(&b).await
                && e.kind() != io::ErrorKind::BrokenPipe
            {
                return Err(e);
            }
        }
        StdinSpec::File(path) => {
            let mut f = fs::File::open(path).await?;
            if let Err(e) = io::copy(&mut f, &mut w).await
                && e.kind() != io::ErrorKind::BrokenPipe
            {
                return Err(e);
            }
        }
        StdinSpec::Null | StdinSpec::Inherit => {}
    }
    let _ = w.shutdown().await;
    Ok(())
}

async fn wait_children(children: &mut [Child]) -> LuaResult<(Vec<i64>, (i64, Option<i64>))> {
    let mut steps = Vec::with_capacity(children.len());
    let mut last = (1, None);
    let last_ix = children.len().saturating_sub(1);
    for (i, ch) in children.iter_mut().enumerate() {
        let status = ch.wait().await.map_err(mlua::Error::external)?;
        let (code, signal) = normalize_status(status);
        steps.push(code);
        if i == last_ix {
            last = (code, signal);
        }
    }
    Ok((steps, last))
}

struct WaitOutcome {
    steps: Vec<i64>,
    code: i64,
    signal: Option<i64>,
    timed_out: bool,
}

async fn wait_children_with_timeout(children: &mut [Child], timeout_ms: Option<u64>) -> LuaResult<WaitOutcome> {
    let Some(ms) = timeout_ms.filter(|v| *v > 0) else {
        let (steps, (code, signal)) = wait_children(children).await?;
        return Ok(WaitOutcome {
            steps,
            code,
            signal,
            timed_out: false,
        });
    };

    if let Ok(res) = time::timeout(Duration::from_millis(ms), wait_children(children)).await {
        let (steps, (code, signal)) = res?;
        Ok(WaitOutcome {
            steps,
            code,
            signal,
            timed_out: false,
        })
    } else {
        // Kill and reap everything to avoid zombies.
        for ch in children.iter_mut() {
            let _ = ch.kill().await;
        }
        for ch in children.iter_mut() {
            let _ = ch.wait().await;
        }

        Ok(WaitOutcome {
            steps: Vec::new(),
            code: 124,
            signal: None,
            timed_out: true,
        })
    }
}

fn normalize_status(status: std::process::ExitStatus) -> (i64, Option<i64>) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.code().map_or_else(
            || {
                status
                    .signal()
                    .map_or((1, None), |sig| (i64::from(128 + sig), Some(i64::from(sig))))
            },
            |code| (i64::from(code), None),
        )
    }
    #[cfg(not(unix))]
    {
        (status.code().unwrap_or(1) as i64, None)
    }
}

fn pipe_value(lhs: CmdSpec, rhs: Value) -> LuaResult<Pipeline> {
    match rhs {
        Value::UserData(ud) => {
            if let Ok(cmd) = ud.borrow::<Cmd>() {
                Ok(Pipeline::new(vec![lhs, cmd.snapshot()], false))
            } else if let Ok(p) = ud.borrow::<Pipeline>() {
                let (p_specs, p_pipefail, p_timeout) = p.snapshot();
                Ok(Pipeline {
                    specs: std::iter::once(lhs).chain(p_specs).collect(),
                    pipefail: p_pipefail,
                    timeout_ms: p_timeout,
                })
            } else {
                Err(mlua::Error::RuntimeError("pipe expects Cmd or Pipeline".into()))
            }
        }
        _ => Err(mlua::Error::RuntimeError("pipe expects Cmd or Pipeline".into())),
    }
}

fn parse_cmd_args(args: Variadic<Value>) -> LuaResult<Vec<String>> {
    // supports:
    //   cmd("git", "status", "--porcelain")
    //   cmd("git", {"status", "--porcelain"})
    fn to_arg(v: Value, ix: usize) -> LuaResult<String> {
        match v {
            Value::String(s) => Ok(s.to_str()?.to_string()),
            Value::Integer(i) => Ok(i.to_string()),
            Value::Number(n) => Ok(n.to_string()),
            other => Err(mlua::Error::RuntimeError(format!(
                "cmd(...): argument #{ix} must be string or number, got {}",
                other.type_name()
            ))),
        }
    }

    if args.len() == 1
        && let Value::Table(t) = &args[0]
    {
        let mut out = Vec::new();
        for (ix0, v) in t.sequence_values::<Value>().enumerate() {
            out.push(to_arg(v?, ix0 + 1)?);
        }
        return Ok(out);
    }

    let mut out = Vec::new();
    for (ix0, v) in args.into_iter().enumerate() {
        out.push(to_arg(v, ix0 + 1)?);
    }
    Ok(out)
}

/// Create the process module
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> LuaResult<Table> {
    let m = lua.create_table()?;
    m.set("cmd", lua.create_function(lua_cmd)?)?;
    m.set("sh", lua.create_function(lua_sh)?)?;
    m.set("exit", lua.create_function(lua_exit)?)?;
    Ok(m)
}

fn lua_cmd(_: &Lua, (prog, args): (String, Variadic<Value>)) -> LuaResult<Cmd> {
    let args = parse_cmd_args(args)?;
    Ok(Cmd::new(prog, args))
}

#[allow(clippy::unnecessary_wraps)]
fn lua_sh(_: &Lua, script: String) -> LuaResult<Cmd> {
    #[cfg(windows)]
    let (prog, args) = ("cmd".to_string(), vec!["/C".to_string(), script]);
    #[cfg(not(windows))]
    let (prog, args) = ("sh".to_string(), vec!["-lc".to_string(), script]);
    Ok(Cmd::new(prog, args))
}

fn lua_exit(_: &Lua, code: Option<i64>) -> LuaResult<()> {
    let mut code = code.unwrap_or(0);
    if code < 0 {
        code = 1;
    }
    let code_i32: i32 = if code > i64::from(i32::MAX) {
        i32::MAX
    } else {
        i32::try_from(code).unwrap_or(0)
    };
    std::process::exit(code_i32);
}
