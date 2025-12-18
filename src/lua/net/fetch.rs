#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::too_many_lines)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use mlua::{Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use reqwest::{Method, redirect};

use tokio::{fs, io::AsyncWriteExt, process::Command, time};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Initializes the `fetch` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let fetch_table = lua.create_table()?;

    // fetch.url(url, opts) -> FetchResponse  (async)
    fetch_table.set(
        "url",
        lua.create_async_function(|_, (url, opts): (String, Value)| async move { fetch_url_async(&url, opts).await })?,
    )?;

    // fetch.git(url, opts) -> FetchResponse  (async)
    fetch_table.set(
        "git",
        lua.create_async_function(|lua, (url, opts): (String, Value)| {
            let overlay = crate::lua::env::overlay_snapshot(&lua);
            async move { fetch_git_async(&url, opts, overlay?).await }
        })?,
    )?;

    Ok(fetch_table)
}

#[derive(Clone, Debug)]
pub struct FetchResponse {
    pub status: i32,
    pub path: Option<String>,
    pub size: u64,
    pub ok: bool,
}

impl UserData for FetchResponse {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_ok", |_, this, ()| Ok(this.ok));
        methods.add_method("status", |_, this, ()| Ok(this.status));
        methods.add_method("path", |_, this, ()| Ok(this.path.clone()));
        methods.add_method("size", |_, this, ()| Ok(this.size));
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("status", |_, this| Ok(this.status));
        fields.add_field_method_get("path", |_, this| Ok(this.path.clone()));
        fields.add_field_method_get("size", |_, this| Ok(this.size));
        fields.add_field_method_get("ok", |_, this| Ok(this.ok));
    }
}

#[derive(Default)]
pub struct UrlOptions {
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Option<Duration>,
    pub follow_redirects: bool,
    pub into: Option<PathBuf>,
    pub max_bytes: Option<u64>,
}

impl UrlOptions {
    /// Creates an `UrlOptions` from a lua table
    /// # Errors [`mlua::Error`]
    pub fn from_value(value: Value) -> mlua::Result<Self> {
        let Value::Table(table) = value else {
            return Ok(Self {
                method: "GET".to_string(),
                follow_redirects: true,
                ..Self::default()
            });
        };

        let mut options = Self {
            method: table
                .get::<Option<String>>("method")?
                .unwrap_or_else(|| "GET".to_string()),
            follow_redirects: table.get::<Option<bool>>("follow_redirects")?.unwrap_or(true),
            ..Self::default()
        };

        if let Some(into) = table.get::<Option<String>>("into")? {
            options.into = Some(PathBuf::from(into));
        }

        if let Ok(headers_table) = table.get::<Table>("headers") {
            for entry in headers_table.pairs::<String, String>() {
                let (key, value) = entry.map_err(mlua::Error::external)?;
                options.headers.push((key, value));
            }
        }

        if let Ok(timeout) = table.get::<Option<f64>>("timeout")
            && let Some(t) = timeout.filter(|t| t.is_sign_positive() && t.is_finite())
        {
            options.timeout = Some(Duration::from_secs_f64(t));
        }

        // max_bytes: integer/number; <=0 disables
        if let Ok(v) = table.get::<Option<Value>>("max_bytes") {
            options.max_bytes = match v {
                #[allow(clippy::cast_sign_loss)]
                Some(Value::Integer(i)) if i > 0 => Some(i as u64),
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(Value::Number(n)) if n.is_finite() && n > 0.0 => Some(n as u64),
                _ => None,
            };
        }

        Ok(options)
    }
}

#[derive(Default)]
pub struct GitOptions {
    pub depth: Option<u32>,
    pub rev: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub recursive: bool,
    pub timeout: Option<Duration>,
    pub into: Option<PathBuf>,
    pub max_bytes: Option<u64>,
    pub filter_blobs: bool,
}

impl GitOptions {
    /// Creates a `GitOptions` from a lua table
    /// # Errors [`mlua::Error`]
    pub fn from_value(value: Value) -> mlua::Result<Self> {
        let Value::Table(table) = value else {
            return Ok(Self {
                depth: Some(1),
                recursive: false,
                ..Self::default()
            });
        };

        let timeout = table
            .get::<Option<f64>>("timeout")?
            .and_then(|t| (t.is_sign_positive() && t.is_finite()).then_some(Duration::from_secs_f64(t)));

        let into = table.get::<Option<String>>("into")?.map(PathBuf::from);
        let max_bytes = match table.get::<Option<Value>>("max_bytes")? {
            #[allow(clippy::cast_sign_loss)]
            Some(Value::Integer(i)) if i > 0 => Some(i as u64),
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Some(Value::Number(n)) if n.is_finite() && n > 0.0 => Some(n as u64),
            _ => None,
        };

        Ok(Self {
            depth: table
                .get::<Option<u32>>("depth")?
                .and_then(|d| (d > 0).then_some(d))
                .or(Some(1)),
            rev: table.get::<Option<String>>("rev")?,
            branch: table.get::<Option<String>>("branch")?,
            tag: table.get::<Option<String>>("tag")?,
            recursive: table.get::<Option<bool>>("recursive")?.unwrap_or(false),
            timeout,
            into,
            max_bytes,
            filter_blobs: table.get::<Option<bool>>("filter_blobs")?.unwrap_or(true),
        })
    }
}

async fn fetch_url_async(url: &str, opts: Value) -> mlua::Result<FetchResponse> {
    let options = UrlOptions::from_value(opts)?;

    let mut client_builder = reqwest::Client::builder();
    if let Some(timeout) = options.timeout {
        client_builder = client_builder.timeout(timeout);
    }
    client_builder = if options.follow_redirects {
        client_builder.redirect(redirect::Policy::limited(10))
    } else {
        client_builder.redirect(redirect::Policy::none())
    };

    let client = client_builder.build().map_err(mlua::Error::external)?;

    let method = options.method.to_uppercase();
    let request_method = match method.as_str() {
        "GET" => Method::GET,
        "DELETE" => Method::DELETE,
        "OPTIONS" => Method::OPTIONS,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "HEAD" => Method::HEAD,
        "PATCH" => Method::PATCH,
        other => return Err(mlua::Error::external(format!("unsupported method: {other}"))),
    };

    let mut req = client.request(request_method, url);
    for (k, v) in &options.headers {
        req = req.header(k, v);
    }

    let resp = req.send().await.map_err(mlua::Error::external)?;
    let status_code = resp.status().as_u16();
    let ok = (200..=299).contains(&status_code);

    // Destination: opts.into or temp path
    let file_path = options.into.clone().unwrap_or_else(|| unique_path("fetch-url"));
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).await.map_err(mlua::Error::external)?;
    }
    let mut file = fs::File::create(&file_path).await.map_err(mlua::Error::external)?;

    let mut size: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(next) = stream.next().await {
        let chunk = next.map_err(mlua::Error::external)?;
        if let Some(max) = options.max_bytes {
            let next_size = size.saturating_add(chunk.len() as u64);
            if next_size > max {
                // best-effort cleanup
                let _ = file.flush().await;
                drop(file);
                let _ = fs::remove_file(&file_path).await;

                let response = FetchResponse {
                    status: 413, // Payload Too Large
                    path: None,
                    size,
                    ok: false,
                };
                return Ok(response);
            }
        }
        size = size.saturating_add(chunk.len() as u64);
        file.write_all(&chunk).await.map_err(mlua::Error::external)?;
    }
    file.flush().await.map_err(mlua::Error::external)?;

    let response = FetchResponse {
        status: i32::from(status_code),
        path: Some(path_to_string(&file_path)),
        size,
        ok,
    };

    Ok(response)
}

async fn fetch_git_async(
    url: &str,
    opts: Value,
    overlay: HashMap<String, Option<String>>,
) -> mlua::Result<FetchResponse> {
    let options = GitOptions::from_value(opts)?;
    let target = options.into.clone().unwrap_or_else(|| unique_path("fetch-git"));
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await.map_err(mlua::Error::external)?;
    }

    let mut clone_args = vec!["clone".to_string()];
    if options.filter_blobs {
        clone_args.push("--filter=blob:none".to_string());
    }
    if let Some(depth) = options.depth {
        clone_args.push("--depth".to_string());
        clone_args.push(depth.to_string());
    }
    if let Some(branch) = options.branch.as_deref().or(options.tag.as_deref()) {
        clone_args.push("--branch".to_string());
        clone_args.push(branch.to_string());
    }
    if options.recursive {
        clone_args.push("--recurse-submodules".to_string());
    }
    clone_args.push(url.to_string());
    clone_args.push(path_to_string(&target));

    let clone_result = run_git_command_async(&clone_args, options.timeout, &overlay).await?;

    if !clone_result.ok {
        let _ = fs::remove_dir_all(&target).await;
        let response = FetchResponse {
            status: clone_result.status,
            path: None,
            size: 0,
            ok: false,
        };
        return Ok(response);
    }

    if let Some(rev) = options.rev {
        let checkout_args = vec!["-C".to_string(), path_to_string(&target), "checkout".to_string(), rev];
        let checkout_result = run_git_command_async(&checkout_args, options.timeout, &overlay).await?;
        if !checkout_result.ok {
            let _ = fs::remove_dir_all(&target).await;
            let response = FetchResponse {
                status: checkout_result.status,
                path: None,
                size: 0,
                ok: false,
            };
            return Ok(response);
        }
    }

    let size = path_size_async(&target).await?;
    if let Some(max) = options.max_bytes
        && size > max
    {
        let _ = fs::remove_dir_all(&target).await;
        let response = FetchResponse {
            status: 413, // Payload Too Large
            path: None,
            size,
            ok: false,
        };
        return Ok(response);
    }

    let response = FetchResponse {
        status: 0,
        path: Some(path_to_string(&target)),
        size,
        ok: true,
    };

    Ok(response)
}

#[derive(Default)]
struct CommandResult {
    status: i32,
    ok: bool,
}

async fn run_git_command_async(
    args: &[String],
    timeout: Option<Duration>,
    overlay: &HashMap<String, Option<String>>,
) -> mlua::Result<CommandResult> {
    let mut cmd = Command::new("git");
    cmd.kill_on_drop(true);
    cmd.args(args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    // Apply Ward-local env overlay to the child process.
    for (k, v) in overlay {
        match v {
            Some(val) => cmd.env(k, val),
            None => cmd.env_remove(k),
        };
    }

    let mut child = cmd.spawn().map_err(mlua::Error::external)?;

    let wait_fut = async {
        let status = child.wait().await.map_err(mlua::Error::external)?;
        let code = status.code().unwrap_or_default();
        Ok::<_, mlua::Error>(CommandResult {
            status: code,
            ok: status.success(),
        })
    };

    if let Some(limit) = timeout {
        if let Ok(r) = time::timeout(limit, wait_fut).await {
            r
        } else {
            // timeout: kill, then wait
            let _ = child.kill().await;
            let status = child.wait().await.map_err(mlua::Error::external)?;
            let code = status.code().unwrap_or_default();
            Ok(CommandResult {
                status: code,
                ok: false,
            })
        }
    } else {
        wait_fut.await
    }
}

fn unique_path(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir();
    temp_dir.join(format!("ward-{prefix}-{timestamp}-{pid}-{counter}", pid = std::process::id()))
}

async fn path_size_async(path: &Path) -> mlua::Result<u64> {
    let meta = fs::symlink_metadata(path).await.map_err(mlua::Error::external)?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    if meta.is_dir() {
        return dir_size_async(path).await;
    }
    Err(mlua::Error::external(format!(
        "unsupported path type: {}",
        path_to_string(path)
    )))
}

async fn dir_size_async(base: &Path) -> mlua::Result<u64> {
    let mut total: u64 = 0;
    let mut stack = vec![base.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut rd = fs::read_dir(&dir).await.map_err(mlua::Error::external)?;
        while let Some(entry) = rd.next_entry().await.map_err(mlua::Error::external)? {
            let meta = entry.metadata().await.map_err(mlua::Error::external)?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }

    Ok(total)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
