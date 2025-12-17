use mlua::{Lua, MetaMethod, Result as LuaResult, Table, UserData, UserDataFields, UserDataMethods, Value, Variadic};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    fs,
    io::{self, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time,
};

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
}

#[derive(Clone)]
struct ProcResult {
    ok: bool,
    code: i64,
    signal: Option<i64>,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
    // optional: per-step codes (useful for debugging)
    steps: Vec<i64>,
}

impl UserData for ProcResult {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("ok", |_, this| Ok(this.ok));
        fields.add_field_method_get("code", |_, this| Ok(this.code));
        fields.add_field_method_get("signal", |_, this| Ok(this.signal));
        fields.add_field_method_get("stdout", |lua, this| {
            Ok(match &this.stdout {
                Some(b) => Some(lua.create_string(b)?),
                None => None,
            })
        });
        fields.add_field_method_get("stderr", |lua, this| {
            Ok(match &this.stderr {
                Some(b) => Some(lua.create_string(b)?),
                None => None,
            })
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
                "{} (code={}, signal={:?})",
                m, this.code, this.signal
            )))
        });
    }
}

impl UserData for Cmd {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // fluent builders
        methods.add_method_mut("cwd", |_, this, path: String| {
            this.spec.cwd = Some(PathBuf::from(path));
            Ok(this.clone())
        });

        methods.add_method_mut("env", |_, this, (k, v): (String, String)| {
            this.spec.env.insert(k, v);
            Ok(this.clone())
        });

        methods.add_method_mut("envs", |_, this, t: Table| {
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                if let (Value::String(ks), Value::String(vs)) = (k, v) {
                    this.spec.env.insert(ks.to_str()?.to_string(), vs.to_str()?.to_string());
                }
            }
            Ok(this.clone())
        });

        #[allow(clippy::cast_sign_loss)]
        methods.add_method_mut("timeout", |_, this, ms: i64| {
            this.spec.timeout_ms = Some(ms.max(0) as u64);
            Ok(this.clone())
        });

        methods.add_method_mut("stdin", |_, this, data: Value| {
            this.spec.stdin = match data {
                Value::String(s) => StdinSpec::Bytes(s.as_bytes().to_vec()),
                _ => StdinSpec::Inherit,
            };
            Ok(this.clone())
        });

        methods.add_method_mut("stdin_file", |_, this, path: String| {
            this.spec.stdin = StdinSpec::File(PathBuf::from(path));
            Ok(this.clone())
        });

        methods.add_method_mut("stderr_to_stdout", |_, this, yes: Option<bool>| {
            this.spec.stderr_to_stdout = yes.unwrap_or(true);
            Ok(this.clone())
        });

        // cmd:pipe(cmd_or_pipeline)
        methods.add_method("pipe", |_, this, rhs: Value| pipe_value(this.spec.clone(), rhs));

        // terminal operations
        methods.add_async_method("run", |_, this, ()| async move {
            run_pipeline(vec![this.spec.clone()], RunMode::Inherit).await
        });

        methods.add_async_method("output", |_, this, ()| async move {
            run_pipeline(vec![this.spec.clone()], RunMode::Capture).await
        });

        // cmd1 | cmd2
        methods.add_meta_method(MetaMethod::BOr, |_, this, rhs: Value| pipe_value(this.spec.clone(), rhs));
    }
}

impl UserData for Pipeline {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("pipe", |_, this, rhs: Value| {
            let mut specs = this.specs.clone();
            match rhs {
                Value::UserData(ud) => {
                    if let Ok(cmd) = ud.borrow::<Cmd>() {
                        specs.push(cmd.spec.clone());
                        Ok(Self { specs })
                    } else if let Ok(p) = ud.borrow::<Self>() {
                        specs.extend(p.specs.clone());
                        Ok(Self { specs })
                    } else {
                        Err(mlua::Error::RuntimeError("pipe(): expected Cmd or Pipeline".into()))
                    }
                }
                _ => Err(mlua::Error::RuntimeError("pipe(): expected Cmd or Pipeline".into())),
            }
        });

        methods.add_async_method("run", |_, this, ()| async move {
            run_pipeline(this.specs.clone(), RunMode::Inherit).await
        });

        methods.add_async_method("output", |_, this, ()| async move {
            run_pipeline(this.specs.clone(), RunMode::Capture).await
        });

        methods.add_meta_method(MetaMethod::BOr, |_, this, rhs: Value| {
            let mut specs = this.specs.clone();
            match rhs {
                Value::UserData(ud) => {
                    if let Ok(cmd) = ud.borrow::<Cmd>() {
                        specs.push(cmd.spec.clone());
                        Ok(Self { specs })
                    } else if let Ok(p) = ud.borrow::<Self>() {
                        specs.extend(p.specs.clone());
                        Ok(Self { specs })
                    } else {
                        Err(mlua::Error::RuntimeError("operator | expects Cmd or Pipeline".into()))
                    }
                }
                _ => Err(mlua::Error::RuntimeError("operator | expects Cmd or Pipeline".into())),
            }
        });
    }
}

fn pipe_value(lhs: CmdSpec, rhs: Value) -> LuaResult<Pipeline> {
    match rhs {
        Value::UserData(ud) => {
            if let Ok(cmd) = ud.borrow::<Cmd>() {
                Ok(Pipeline {
                    specs: vec![lhs, cmd.spec.clone()],
                })
            } else if let Ok(p) = ud.borrow::<Pipeline>() {
                let mut specs = vec![lhs];
                specs.extend(p.specs.clone());
                Ok(Pipeline { specs })
            } else {
                Err(mlua::Error::RuntimeError("pipe expects Cmd or Pipeline".into()))
            }
        }
        _ => Err(mlua::Error::RuntimeError("pipe expects Cmd or Pipeline".into())),
    }
}

enum RunMode {
    Inherit,
    Capture,
}

async fn pump_to_shared_stdin<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    writer: Arc<Mutex<ChildStdin>>,
) {
    let mut buf = [0u8; 16 * 1024];
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

async fn feed_first_stdin(child: &mut Child, stdin: StdinSpec) {
    let Some(mut w) = child.stdin.take() else {
        return;
    };

    match stdin {
        StdinSpec::Inherit => {}
        StdinSpec::Bytes(b) => {
            let _ = w.write_all(&b).await;
        }
        StdinSpec::File(path) => {
            if let Ok(mut f) = fs::File::open(path).await {
                let _ = io::copy(&mut f, &mut w).await;
            }
        }
    }
    // close stdin
    drop(w);
}

fn apply_common_opts(cmd: &mut Command, spec: &CmdSpec) {
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    if !spec.env.is_empty() {
        cmd.envs(&spec.env);
    }
}

#[allow(clippy::too_many_lines)]
async fn run_pipeline(specs: Vec<CmdSpec>, mode: RunMode) -> LuaResult<ProcResult> {
    if specs.is_empty() {
        return Err(mlua::Error::RuntimeError("empty pipeline".into()));
    }

    let n = specs.len();

    // Spawn all children with the correct stdio shapes.
    let mut children: Vec<Child> = Vec::with_capacity(n);

    for (i, spec) in specs.iter().enumerate() {
        let mut c = Command::new(&spec.program);
        c.args(&spec.args);
        apply_common_opts(&mut c, spec);

        // stdin:
        // - first: piped if we need to feed bytes/file, otherwise inherit
        // - others: piped to accept upstream
        if i == 0 {
            match spec.stdin {
                StdinSpec::Inherit => {
                    c.stdin(std::process::Stdio::inherit());
                }
                _ => {
                    c.stdin(std::process::Stdio::piped());
                }
            }
        } else {
            c.stdin(std::process::Stdio::piped());
        }

        // stdout:
        // - intermediate: always piped
        // - last: inherit for RunMode::Inherit, piped for RunMode::Capture
        if i < n - 1 {
            c.stdout(std::process::Stdio::piped());
        } else {
            match mode {
                RunMode::Inherit => c.stdout(std::process::Stdio::inherit()),
                RunMode::Capture => c.stdout(std::process::Stdio::piped()),
            };
        }

        // stderr:
        // - if stderr_to_stdout: pipe it so we can feed into next or merge into captured stdout
        // - else: inherit (or capture in Capture mode for last command)
        if spec.stderr_to_stdout {
            c.stderr(std::process::Stdio::piped());
        } else if i == n - 1 {
            match mode {
                RunMode::Inherit => c.stderr(std::process::Stdio::inherit()),
                RunMode::Capture => c.stderr(std::process::Stdio::piped()),
            };
        } else {
            // intermediate stderr inherits (shell-like)
            c.stderr(std::process::Stdio::inherit());
        }

        let child = c.spawn().map_err(mlua::Error::external)?;
        children.push(child);
    }

    // Wire pipes: stdout(i) -> stdin(i+1). If stderr_to_stdout(i), also stderr(i) -> stdin(i+1).
    let mut link_tasks = Vec::new();

    // Take all stdio handles we need.
    let mut stdins: Vec<Option<ChildStdin>> = children.iter_mut().map(|ch| ch.stdin.take()).collect();
    let mut stdouts: Vec<Option<ChildStdout>> = children.iter_mut().map(|ch| ch.stdout.take()).collect();
    let mut stderrs = Vec::with_capacity(n);
    for (i, spec) in specs.iter().enumerate() {
        if spec.stderr_to_stdout || (i == n - 1 && matches!(mode, RunMode::Capture)) {
            stderrs.push(children[i].stderr.take());
        } else {
            stderrs.push(None);
        }
    }

    for i in 0..(n - 1) {
        let out = stdouts[i]
            .take()
            .ok_or_else(|| mlua::Error::RuntimeError("missing stdout for pipe".into()))?;
        let in_next = stdins[i + 1]
            .take()
            .ok_or_else(|| mlua::Error::RuntimeError("missing stdin for pipe".into()))?;

        let shared = Arc::new(Mutex::new(in_next));
        link_tasks.push(tokio::spawn(pump_to_shared_stdin(out, shared.clone())));

        if specs[i].stderr_to_stdout
            && let Some(err) = stderrs[i].take()
        {
            link_tasks.push(tokio::spawn(pump_to_shared_stdin(err, shared)));
        }
    }

    // Feed stdin of first command if configured (bytes/file).
    feed_first_stdin(&mut children[0], specs[0].stdin.clone()).await;

    // Capture last stdout/stderr if required.
    let mut last_stdout_task = None;
    let mut last_stderr_task = None;
    let mut last_stderr_to_stdout_task = None;

    if matches!(mode, RunMode::Capture) {
        if let Some(out) = stdouts[n - 1].take() {
            last_stdout_task = Some(tokio::spawn(async move {
                let mut r = out;
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf).await;
                buf
            }));
        }
        if let Some(err) = stderrs[n - 1].take() {
            last_stderr_task = Some(tokio::spawn(async move {
                let mut r = err;
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf).await;
                buf
            }));
        }
    } else if specs[n - 1].stderr_to_stdout {
        // In inherit mode, leaving stderr piped without reading it can deadlock the child.
        // Best-effort: drain the last stderr and forward it to the parent stdout.
        if let Some(err) = stderrs[n - 1].take() {
            last_stderr_to_stdout_task = Some(tokio::spawn(async move {
                let mut r = err;
                let mut out = tokio::io::stdout();
                let mut buf = [0u8; 16 * 1024];
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
    }

    // Wait with optional timeout (use last command’s timeout if set)
    let timeout_ms = specs[n - 1].timeout_ms;
    let wait_fut = async {
        let mut steps = Vec::with_capacity(n);
        let mut last_status = None;

        for (i, ch) in children.iter_mut().enumerate() {
            let status = ch.wait().await.map_err(mlua::Error::external)?;
            let (code, signal) = normalize_status(status);
            steps.push(code);
            if i == n - 1 {
                last_status = Some((code, signal));
            }
        }

        Ok::<_, mlua::Error>((steps, last_status.unwrap_or((1, None))))
    };

    let (steps, (code, signal)) = if let Some(ms) = timeout_ms.filter(|m| *m > 0) {
        if let Ok(v) = time::timeout(Duration::from_millis(ms), wait_fut).await {
            v?
        } else {
            // timeout: kill all and reap children to avoid zombies.
            for ch in &mut children {
                let _ = ch.kill().await;
            }
            for ch in &mut children {
                let _ = ch.wait().await;
            }

            // Ensure background pipe/capture tasks don't outlive this call.
            for t in &link_tasks {
                t.abort();
            }
            for t in link_tasks {
                let _ = t.await;
            }

            if let Some(t) = last_stdout_task.take() {
                t.abort();
                let _ = t.await;
            }
            if let Some(t) = last_stderr_task.take() {
                t.abort();
                let _ = t.await;
            }
            if let Some(t) = last_stderr_to_stdout_task.take() {
                t.abort();
                let _ = t.await;
            }

            return Ok(ProcResult {
                ok: false,
                code: 124,
                signal: None,
                stdout: None,
                stderr: None,
                steps: vec![],
            });
        }
    } else {
        wait_fut.await?
    };

    // Make sure pipe tasks stop
    for t in link_tasks {
        let _ = t.await;
    }

    if let Some(t) = last_stderr_to_stdout_task {
        let _ = t.await;
    }

    let mut stdout = match last_stdout_task {
        Some(t) => t.await.ok(),
        None => None,
    };
    let stderr = match last_stderr_task {
        Some(t) => t.await.ok(),
        None => None,
    };

    // If last command has stderr_to_stdout and we captured, merge stderr into stdout (basic 2>&1 behavior).
    if matches!(mode, RunMode::Capture)
        && specs[n - 1].stderr_to_stdout
        && let Some(e) = &stderr
    {
        if stdout.is_none() {
            stdout = Some(e.clone());
        } else if let Some(o) = &mut stdout {
            o.extend_from_slice(e);
        }
    }

    Ok(ProcResult {
        ok: code == 0,
        code,
        signal,
        stdout,
        stderr,
        steps,
    })
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

fn parse_cmd_args(args: Variadic<Value>) -> LuaResult<Vec<String>> {
    // supports:
    //   cmd("git", "status", "--porcelain")
    //   cmd("git", {"status", "--porcelain"})
    if args.len() == 1
        && let Value::Table(t) = &args[0]
    {
        let mut out = Vec::new();
        for v in t.sequence_values::<Value>() {
            match v? {
                Value::String(s) => out.push(s.to_str()?.to_string()),
                Value::Integer(i) => out.push(i.to_string()),
                Value::Number(n) => out.push(n.to_string()),
                _ => {}
            }
        }
        return Ok(out);
    }

    let mut out = Vec::new();
    for v in args {
        match v {
            Value::String(s) => out.push(s.to_str()?.to_string()),
            Value::Integer(i) => out.push(i.to_string()),
            Value::Number(n) => out.push(n.to_string()),
            _ => {}
        }
    }
    Ok(out)
}

/// Create the process module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> LuaResult<Table> {
    let m = lua.create_table()?;

    // cmd(prog, ...args) -> Cmd
    m.set(
        "cmd",
        lua.create_function(|_, (prog, args): (String, Variadic<Value>)| {
            let args = parse_cmd_args(args)?;
            Ok(Cmd {
                spec: CmdSpec {
                    program: prog,
                    args,
                    ..Default::default()
                },
            })
        })?,
    )?;

    // sh("...") -> Cmd  (explicit shell mode)
    m.set(
        "sh",
        lua.create_function(|_, script: String| {
            #[cfg(windows)]
            let (prog, args) = ("cmd".to_string(), vec!["/C".to_string(), script]);
            #[cfg(not(windows))]
            let (prog, args) = ("sh".to_string(), vec!["-lc".to_string(), script]);

            Ok(Cmd {
                spec: CmdSpec {
                    program: prog,
                    args,
                    ..Default::default()
                },
            })
        })?,
    )?;

    Ok(m)
}
