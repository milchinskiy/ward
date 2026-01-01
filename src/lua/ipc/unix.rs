#![allow(clippy::needless_pass_by_value, clippy::cast_possible_truncation)]

use mlua::{Lua, MetaMethod, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value};

#[cfg(unix)]
use std::{
    fs::Permissions as FsPermissions,
    os::unix::{fs::PermissionsExt, net::UnixListener as StdUnixListener},
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(unix)]
use socket2::{Domain, Socket, Type};
#[cfg(unix)]
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        UnixListener, UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::Mutex as AsyncMutex,
};

#[cfg(not(unix))]
pub fn define(_lua: &Lua) -> mlua::Result<Table> {
    Err(mlua::Error::external("ward.ipc.unix is only available on Unix platforms"))
}

#[cfg(unix)]
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let unix = lua.create_table()?;

    unix.set(
        "connect",
        lua.create_async_function(|lua, (path, opts): (String, Option<Table>)| async move {
            connect(&lua, PathBuf::from(path), opts).await
        })?,
    )?;

    unix.set(
        "listen",
        lua.create_async_function(|lua, (path, opts): (String, Option<Table>)| async move {
            listen(&lua, PathBuf::from(path), opts).await
        })?,
    )?;

    Ok(unix)
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct PermSpec {
    mode: Option<u32>,
    owner: Option<u32>,
    group: Option<u32>,
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct ListenOptions {
    backlog: Option<i32>,
    unlink: bool,
    unlink_on_close: bool,
    mkdir: bool,
    perms: PermSpec,
}

#[cfg(unix)]
impl ListenOptions {
    fn parse(v: Option<Table>) -> mlua::Result<Self> {
        let Some(t) = v else {
            return Ok(Self {
                backlog: None,
                unlink: true,
                unlink_on_close: true,
                mkdir: false,
                perms: PermSpec {
                    mode: None,
                    owner: None,
                    group: None,
                },
            });
        };

        let backlog = t.get::<Option<Value>>("backlog")?.map(parse_backlog).transpose()?;
        let mode = t.get::<Option<Value>>("mode")?.map(parse_mode).transpose()?;
        let owner = t.get::<Option<Value>>("owner")?.map(parse_uid_gid).transpose()?;
        let group = t.get::<Option<Value>>("group")?.map(parse_uid_gid).transpose()?;
        let unlink = t.get::<Option<bool>>("unlink")?.unwrap_or(true);
        let unlink_on_close = t.get::<Option<bool>>("unlink_on_close")?.unwrap_or(true);
        let mkdir = t.get::<Option<bool>>("mkdir")?.unwrap_or(false);

        Ok(Self {
            backlog,
            unlink,
            unlink_on_close,
            mkdir,
            perms: PermSpec { mode, owner, group },
        })
    }
}

#[cfg(unix)]
fn parse_backlog(v: Value) -> mlua::Result<i32> {
    match v {
        Value::Integer(i) if i > 0 => i32::try_from(i).map_err(|_| mlua::Error::external("backlog is too large")),
        Value::Number(n) if n.is_finite() && n > 0.0 => {
            i32::try_from(n as i64).map_err(|_| mlua::Error::external("backlog is too large or fractional"))
        }
        _ => Err(mlua::Error::external("backlog must be a positive integer")),
    }
}

#[cfg(unix)]
fn parse_mode(v: Value) -> mlua::Result<u32> {
    match v {
        Value::Integer(i) if (0..=0o7777).contains(&i) => Ok(u32::try_from(i).unwrap_or(0)),
        Value::Number(n) if n.is_finite() && n >= 0.0 => {
            let i = n as i64;
            if (0..=0o7777).contains(&i) {
                Ok(u32::try_from(i).unwrap_or(0))
            } else {
                Err(mlua::Error::external("mode must be between 0 and 0o7777"))
            }
        }
        _ => Err(mlua::Error::external("mode must be an integer (octal) between 0 and 0o7777")),
    }
}

#[cfg(unix)]
fn parse_uid_gid(v: Value) -> mlua::Result<u32> {
    match v {
        Value::Integer(i) if i >= 0 => u32::try_from(i).map_err(|_| mlua::Error::external("id out of range")),
        Value::Number(n) if n.is_finite() && n >= 0.0 => {
            u32::try_from(n as i64).map_err(|_| mlua::Error::external("id out of range"))
        }
        _ => Err(mlua::Error::external("owner/group must be a non-negative integer")),
    }
}

#[cfg(unix)]
async fn connect(_lua: &Lua, path: PathBuf, opts: Option<Table>) -> mlua::Result<UnixStreamUserData> {
    if opts.is_some() {
        // Reserved for future options (e.g., timeouts). Avoid silently accepting typos.
        return Err(mlua::Error::external("connect options are not supported yet"));
    }

    let stream = UnixStream::connect(path)
        .await
        .map_err(|e| mlua::Error::external(format!("connect failed: {e}")))?;
    Ok(UnixStreamUserData::new(stream))
}

#[cfg(unix)]
async fn listen(_lua: &Lua, path: PathBuf, opts: Option<Table>) -> mlua::Result<UnixListenerUserData> {
    let options = ListenOptions::parse(opts)?;
    let parent = path.parent().map(Path::to_path_buf);
    if options.mkdir
        && let Some(ref dir) = parent
    {
        fs::create_dir_all(dir)
            .await
            .map_err(|e| mlua::Error::external(format!("create dir failed: {e}")))?;
    }

    let exists = fs::metadata(&path).await.is_ok();
    if exists {
        if !options.unlink {
            return Err(mlua::Error::external("address in use (unlink disabled)"));
        }

        match UnixStream::connect(path.clone()).await {
            Ok(_) => {
                return Err(mlua::Error::external("address in use"));
            }
            Err(e) => match e.kind() {
                std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::NotFound
                | std::io::ErrorKind::AddrNotAvailable => {
                    // stale; fall through and remove
                }
                _ => return Err(mlua::Error::external(format!("cannot verify stale socket: {e}"))),
            },
        }

        // Best-effort stale removal.
        if let Err(e) = fs::remove_file(&path).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(mlua::Error::external(format!("failed to remove stale socket: {e}")));
        }
    }

    let addr =
        socket2::SockAddr::unix(&path).map_err(|e| mlua::Error::external(format!("invalid socket path: {e}")))?;
    let sock = Socket::new(Domain::UNIX, Type::STREAM, None)
        .map_err(|e| mlua::Error::external(format!("socket create failed: {e}")))?;
    sock.set_nonblocking(true)
        .map_err(|e| mlua::Error::external(format!("socket nonblocking failed: {e}")))?;
    sock.bind(&addr)
        .map_err(|e| mlua::Error::external(format!("bind failed: {e}")))?;
    sock.listen(options.backlog.unwrap_or(128))
        .map_err(|e| mlua::Error::external(format!("listen failed: {e}")))?;

    let std_listener: StdUnixListener = sock.into();
    std_listener
        .set_nonblocking(true)
        .map_err(|e| mlua::Error::external(format!("set_nonblocking failed: {e}")))?;

    let listener = UnixListener::from_std(std_listener)
        .map_err(|e| mlua::Error::external(format!("listener init failed: {e}")))?;

    if let Some(mode) = options.perms.mode {
        let perms = FsPermissions::from_mode(mode);
        fs::set_permissions(&path, perms)
            .await
            .map_err(|e| mlua::Error::external(format!("chmod failed: {e}")))?;
    }

    if options.perms.owner.is_some() || options.perms.group.is_some() {
        use nix::unistd::{Gid, Uid};

        let uid = options.perms.owner.map(Uid::from_raw);
        let gid = options.perms.group.map(Gid::from_raw);

        nix::unistd::chown(&path, uid, gid).map_err(|e| mlua::Error::external(format!("chown failed: {e}")))?;
    }

    Ok(UnixListenerUserData::new(listener, path, options.unlink_on_close, true))
}

#[cfg(unix)]
#[derive(Clone)]
struct UnixStreamUserData {
    reader: Arc<AsyncMutex<Option<OwnedReadHalf>>>,
    writer: Arc<AsyncMutex<Option<OwnedWriteHalf>>>,
}

#[cfg(unix)]
impl UnixStreamUserData {
    fn new(stream: UnixStream) -> Self {
        let (reader, writer) = stream.into_split();
        Self {
            reader: Arc::new(AsyncMutex::new(Some(reader))),
            writer: Arc::new(AsyncMutex::new(Some(writer))),
        }
    }

    async fn read_chunk(
        lua: &Lua,
        reader: Arc<AsyncMutex<Option<OwnedReadHalf>>>,
        n: usize,
    ) -> mlua::Result<MultiValue> {
        let mut guard = reader.lock().await;
        let Some(ref mut r) = guard.as_mut() else {
            drop(guard);
            return mv_err(lua, "closed");
        };

        let mut buf = vec![0_u8; n];
        match r.read(&mut buf).await {
            Ok(0) => mv_err(lua, "eof"),
            Ok(sz) => {
                buf.truncate(sz);
                mv_ok(lua, buf)
            }
            Err(e) => mv_err(lua, e),
        }
    }

    async fn read_exact(
        lua: &Lua,
        reader: Arc<AsyncMutex<Option<OwnedReadHalf>>>,
        n: usize,
    ) -> mlua::Result<MultiValue> {
        let mut buf = vec![0_u8; n];

        let mut guard = reader.lock().await;
        let Some(ref mut r) = guard.as_mut() else {
            drop(guard);
            return mv_err(lua, "closed");
        };

        match r.read_exact(&mut buf).await {
            Ok(_) => mv_ok(lua, buf),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => mv_err(lua, "eof"),
            Err(e) => mv_err(lua, e),
        }
    }

    async fn write(
        lua: &Lua,
        writer: Arc<AsyncMutex<Option<OwnedWriteHalf>>>,
        data: &[u8],
    ) -> mlua::Result<MultiValue> {
        let mut guard = writer.lock().await;
        let Some(ref mut w) = guard.as_mut() else {
            drop(guard);
            return mv_err(lua, "closed");
        };

        match w.write(data).await {
            Ok(sz) => {
                let mut mv = MultiValue::new();
                mv.push_back(Value::Integer(i64::try_from(sz).unwrap_or(i64::MAX)));
                Ok(mv)
            }
            Err(e) => mv_err(lua, e),
        }
    }

    async fn write_all(
        lua: &Lua,
        writer: Arc<AsyncMutex<Option<OwnedWriteHalf>>>,
        data: &[u8],
    ) -> mlua::Result<MultiValue> {
        let mut guard = writer.lock().await;
        let Some(ref mut w) = guard.as_mut() else {
            drop(guard);
            return mv_err(lua, "closed");
        };

        match w.write_all(data).await {
            Ok(()) => {
                let mut mv = MultiValue::new();
                mv.push_back(Value::Boolean(true));
                Ok(mv)
            }
            Err(e) => mv_err(lua, e),
        }
    }

    async fn shutdown(lua: &Lua, writer: Arc<AsyncMutex<Option<OwnedWriteHalf>>>) -> mlua::Result<MultiValue> {
        let mut guard = writer.lock().await;
        if let Some(ref mut w) = guard.as_mut()
            && let Err(e) = w.shutdown().await
        {
            drop(guard);
            return mv_err(lua, e);
        }
        Ok(MultiValue::new())
    }

    async fn close(&self) -> bool {
        let mut r = self.reader.lock().await;
        let mut w = self.writer.lock().await;
        *r = None;
        drop(r);
        *w = None;
        true
    }
}

#[cfg(unix)]
impl UserData for UnixStreamUserData {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("closed", |_, this| {
            Ok(this.reader.try_lock().is_ok_and(|g| g.is_none()) || this.writer.try_lock().is_ok_and(|g| g.is_none()))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("read", |lua, this, n: Option<i64>| async move {
            let n = n.unwrap_or(16 * 1024);
            if n <= 0 {
                return Err(mlua::Error::external("read(n): n must be > 0"));
            }
            let n = usize::try_from(n).map_err(|_| mlua::Error::external("read(n): n too large"))?;
            Self::read_chunk(&lua, this.reader.clone(), n).await
        });

        methods.add_async_method("read_exact", |lua, this, n: i64| async move {
            if n <= 0 {
                return Err(mlua::Error::external("read_exact(n): n must be > 0"));
            }
            let n = usize::try_from(n).map_err(|_| mlua::Error::external("read_exact(n): n too large"))?;
            Self::read_exact(&lua, this.reader.clone(), n).await
        });

        methods.add_async_method("write", |lua, this, data: Value| async move {
            let Value::String(s) = data else {
                return Err(mlua::Error::external("write(bytes): expected bytes string"));
            };
            let bytes = s.as_bytes();
            Self::write(&lua, this.writer.clone(), bytes.as_ref()).await
        });

        methods.add_async_method("write_all", |lua, this, data: Value| async move {
            let Value::String(s) = data else {
                return Err(mlua::Error::external("write_all(bytes): expected bytes string"));
            };
            let bytes = s.as_bytes();
            Self::write_all(&lua, this.writer.clone(), bytes.as_ref()).await
        });

        methods.add_async_method("shutdown", |lua, this, ()| async move {
            Self::shutdown(&lua, this.writer.clone()).await
        });

        methods.add_async_method("close", |_, this, ()| async move { Ok(this.close().await) });

        methods.add_async_method("wait", |lua, this, n: Option<i64>| async move {
            let n = n.unwrap_or(16 * 1024);
            if n <= 0 {
                return Err(mlua::Error::external("wait(n): n must be > 0"));
            }
            let n = usize::try_from(n).map_err(|_| mlua::Error::external("wait(n): n too large"))?;
            Self::read_chunk(&lua, this.reader.clone(), n).await
        });

        methods.add_async_meta_method(MetaMethod::Call, |lua, this, n: Option<i64>| async move {
            let n = n.unwrap_or(16 * 1024);
            if n <= 0 {
                return Err(mlua::Error::external("__call(n): n must be > 0"));
            }
            let n = usize::try_from(n).map_err(|_| mlua::Error::external("__call(n): n too large"))?;
            Self::read_chunk(&lua, this.reader.clone(), n).await
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("UnixStream()".to_string()));
    }
}

#[cfg(unix)]
#[derive(Clone)]
struct UnixListenerUserData {
    inner: Arc<AsyncMutex<Option<UnixListener>>>,
    path: PathBuf,
    unlink_on_close: bool,
    own_path: bool,
}

#[cfg(unix)]
impl UnixListenerUserData {
    fn new(listener: UnixListener, path: PathBuf, unlink_on_close: bool, own_path: bool) -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Some(listener))),
            path,
            unlink_on_close,
            own_path,
        }
    }

    async fn close_inner(&self) -> mlua::Result<bool> {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            return Ok(false);
        }
        *guard = None;
        drop(guard);

        if self.unlink_on_close
            && self.own_path
            && let Err(e) = fs::remove_file(&self.path).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(mlua::Error::external(format!("unlink failed: {e}")));
        }

        Ok(true)
    }
}

#[cfg(unix)]
impl Drop for UnixListenerUserData {
    fn drop(&mut self) {
        if !self.unlink_on_close || !self.own_path {
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let path = self.path.clone();
            handle.spawn(async move {
                let _ = fs::remove_file(path).await;
            });
        }
    }
}

#[cfg(unix)]
impl UserData for UnixListenerUserData {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("accept", |lua, this, ()| async move {
            let mut guard = this.inner.lock().await;
            let Some(listener) = guard.as_mut() else {
                drop(guard);
                return mv_err(&lua, "closed");
            };

            match listener.accept().await {
                Ok((stream, _)) => {
                    let stream_ud = UnixStreamUserData::new(stream);
                    let mut mv = MultiValue::new();
                    mv.push_back(Value::UserData(lua.create_userdata(stream_ud)?));
                    Ok(mv)
                }
                Err(e) => mv_err(&lua, e),
            }
        });

        methods.add_async_method("close", |_, this, ()| async move { this.close_inner().await });

        methods.add_meta_method(MetaMethod::ToString, |_, _this, ()| Ok("UnixListener()".to_string()));
    }
}

#[cfg(unix)]
fn mv_err(lua: &Lua, err: impl std::fmt::Display) -> mlua::Result<MultiValue> {
    let mut mv = MultiValue::new();
    mv.push_back(Value::Nil);
    mv.push_back(Value::String(lua.create_string(err.to_string())?));
    Ok(mv)
}

#[cfg(unix)]
fn mv_ok(lua: &Lua, buf: Vec<u8>) -> mlua::Result<MultiValue> {
    let mut mv = MultiValue::new();
    mv.push_back(Value::String(lua.create_string(&buf)?));
    Ok(mv)
}
