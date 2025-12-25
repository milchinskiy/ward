#![allow(clippy::too_many_lines)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unnecessary_wraps)]

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use filetime::FileTime;
use glob::glob;
use mlua::{Lua, Table, Value};
use tokio::io::AsyncWriteExt;

pub mod path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

#[cfg(unix)]
use nix::unistd::{Gid, Uid, chown as nix_chown};

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(target_os = "windows")]
use std::os::windows::fs::{symlink_dir, symlink_file};

#[cfg(unix)]
fn symlink_platform(source: &Path, dest: &Path) -> std::io::Result<()> {
    symlink(source, dest)
}

#[cfg(target_os = "windows")]
fn symlink_platform(source: &Path, dest: &Path) -> std::io::Result<()> {
    // Windows differentiates between file and directory symlinks.
    // Best-effort: detect the source type; if it doesn't exist, fall back to file symlink.
    match std::fs::metadata(source) {
        Ok(meta) if meta.is_dir() => symlink_dir(source, dest),
        Ok(_) => symlink_file(source, dest),
        Err(_) => symlink_file(source, dest),
    }
}

#[derive(Debug, Clone)]
struct OpOutcome {
    ok: bool,
    err: Option<String>,
}

impl OpOutcome {
    #[allow(clippy::missing_const_for_fn)]
    fn ok() -> Self {
        Self { ok: true, err: None }
    }

    fn fail<E: std::fmt::Display>(e: E) -> Self {
        Self {
            ok: false,
            err: Some(e.to_string()),
        }
    }

    fn fail_msg(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            err: Some(msg.into()),
        }
    }
}

fn op_table(lua: &Lua, out: OpOutcome) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("ok", out.ok)?;
    t.set("err", out.err)?;
    Ok(t)
}

/// Initializes the `fs` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let fs_table = lua.create_table()?;

    // Pure path manipulation helpers (no filesystem access)
    {
        let path_mod = path::define(lua)?;
        fs_table.set("path", path_mod.clone())?;
        lua.register_module("ward.fs.path", path_mod)?;
    }

    fs_table.set(
        "is_exists",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(exists_async(&path).await)
        })?,
    )?;

    fs_table.set(
        "is_dir",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(tokio::fs::metadata(&path).await.map(|m| m.is_dir()).unwrap_or(false))
        })?,
    )?;

    fs_table.set(
        "is_file",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(tokio::fs::metadata(&path).await.map(|m| m.is_file()).unwrap_or(false))
        })?,
    )?;

    fs_table.set(
        "is_link",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_symlink_async(&path).await)
        })?,
    )?;
    fs_table.set(
        "is_symlink",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_symlink_async(&path).await)
        })?,
    )?;

    fs_table.set(
        "is_block_device",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_type_async(path.as_path(), FileKind::Block).await)
        })?,
    )?;
    fs_table.set(
        "is_char_device",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_type_async(path.as_path(), FileKind::Char).await)
        })?,
    )?;
    fs_table.set(
        "is_fifo",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_type_async(path.as_path(), FileKind::Fifo).await)
        })?,
    )?;
    fs_table.set(
        "is_socket",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_type_async(path.as_path(), FileKind::Socket).await)
        })?,
    )?;

    fs_table.set(
        "is_executable",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_executable_async(path.as_path()).await)
        })?,
    )?;

    fs_table.set(
        "is_readable",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_readable_path_async(path.as_path()).await)
        })?,
    )?;

    fs_table.set(
        "is_writable",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            Ok(is_writable_path_async(path.as_path()).await)
        })?,
    )?;

    fs_table.set(
        "readlink",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            readlink_async(path.as_path()).await
        })?,
    )?;

    fs_table.set(
        "realpath",
        lua.create_async_function(|_, path: Value| async move {
            let path = value_to_path_buf(path)?;
            realpath_async(path.as_path()).await
        })?,
    )?;

    // pure helpers can stay sync
    fs_table.set(
        "dirname",
        lua.create_function(|_, path: Value| {
            let path = value_to_path_buf(path)?;
            Ok(dirname(path.as_path()))
        })?,
    )?;
    fs_table.set(
        "basename",
        lua.create_function(|_, path: Value| {
            let path = value_to_path_buf(path)?;
            Ok(basename(path.as_path()))
        })?,
    )?;

    // list/glob return Vec<String> (Lua sees a sequence table in typical mlua conversions)
    fs_table.set(
        "list",
        lua.create_async_function(|_, (path, opts): (Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = ListOpts::from_value(opts)?;
            list_async(path.as_path(), opts).await
        })?,
    )?;

    fs_table.set(
        "glob",
        lua.create_async_function(|_, pattern: String| async move {
            let matches: Vec<String> = tokio::task::spawn_blocking(move || glob_paths(pattern))
                .await
                .map_err(mlua::Error::external)? // JoinError
                .map_err(mlua::Error::external)?; // String -> mlua::Error

            Ok(matches)
        })?,
    )?;

    fs_table.set(
        "join",
        lua.create_function(|_, (path, rest): (Value, mlua::Variadic<Value>)| {
            let path = value_to_path_buf(path)?;
            join(path, rest)
        })?,
    )?;

    fs_table.set(
        "mkdir",
        lua.create_async_function(|lua, (path, opts): (Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = MkdirOpts::from_value(opts)?;
            op_table(&lua, mkdir_async(path.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "rm",
        lua.create_async_function(|lua, (path, opts): (Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = RemoveOpts::from_value(opts)?;
            op_table(&lua, rm_async(path.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "unlink",
        lua.create_async_function(|lua, (path, opts): (Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = ForceOnly::from_value(opts)?;
            op_table(&lua, unlink_async(path.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "chmod",
        lua.create_async_function(|lua, (path, mode, opts): (Value, u32, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = RecursiveForce::from_value(opts)?;
            op_table(&lua, chmod_async(path.as_path(), mode, opts).await)
        })?,
    )?;

    fs_table.set(
        "chown",
        lua.create_async_function(|lua, (path, uid, gid, opts): (Value, u32, u32, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = RecursiveForce::from_value(opts)?;
            op_table(&lua, chown_async(path.as_path(), uid, gid, opts).await)
        })?,
    )?;

    fs_table.set(
        "rename",
        lua.create_async_function(|lua, (old_path, new_path, opts): (Value, Value, Value)| async move {
            let old = value_to_path_buf(old_path)?;
            let new = value_to_path_buf(new_path)?;
            let opts = ForceOnly::from_value(opts)?;
            op_table(&lua, rename_async(old.as_path(), new.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "link",
        lua.create_async_function(|lua, (old_path, new_path, opts): (Value, Value, Value)| async move {
            let old = value_to_path_buf(old_path)?;
            let new = value_to_path_buf(new_path)?;
            let opts = ForceOnly::from_value(opts)?;
            op_table(&lua, link_async(old.as_path(), new.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "symlink",
        lua.create_async_function(|lua, (old_path, new_path, opts): (Value, Value, Value)| async move {
            let old = value_to_path_buf(old_path)?;
            let new = value_to_path_buf(new_path)?;
            let opts = ForceOnly::from_value(opts)?;
            op_table(&lua, symlink_path_async(old.as_path(), new.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "touch",
        lua.create_async_function(|lua, (path, opts): (Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = TouchOpts::from_value(opts)?;
            op_table(&lua, touch_async(path.as_path(), opts).await)
        })?,
    )?;

    // IMPORTANT: read returns Vec<u8> (binary-safe); for Text mode we validate UTF-8 but still return bytes.
    fs_table.set(
        "read",
        lua.create_async_function(|_, (path, opts): (Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = ReadOpts::from_value(opts)?;
            read_async(path.as_path(), opts).await
        })?,
    )?;

    fs_table.set(
        "write",
        lua.create_async_function(|lua, (path, data, opts): (Value, mlua::Value, Value)| async move {
            let path = value_to_path_buf(path)?;
            let opts = WriteOpts::from_value(opts)?;

            // Convert mlua::Value BEFORE any await
            let bytes = if opts.binary {
                value_to_bytes(data)?
            } else {
                value_to_string(data)?.into_bytes()
            };

            op_table(&lua, write_async(path.as_path(), bytes, opts).await)
        })?,
    )?;

    fs_table.set(
        "copy",
        lua.create_async_function(|lua, (from, to, opts): (Value, Value, Value)| async move {
            let from = value_to_path_buf(from)?;
            let to = value_to_path_buf(to)?;
            let opts = ForceOnly::from_value(opts)?;
            op_table(&lua, copy_async(from.as_path(), to.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "move",
        lua.create_async_function(|lua, (from, to, opts): (Value, Value, Value)| async move {
            let from = value_to_path_buf(from)?;
            let to = value_to_path_buf(to)?;
            let opts = ForceOnly::from_value(opts)?;
            op_table(&lua, move_async(from.as_path(), to.as_path(), opts).await)
        })?,
    )?;

    fs_table.set(
        "tempdir",
        lua.create_async_function(|_, prefix: Option<String>| async move { tempdir_async(prefix).await })?,
    )?;

    Ok(fs_table)
}

#[derive(Copy, Clone)]
enum FileKind {
    Block,
    Char,
    Fifo,
    Socket,
}

async fn exists_async(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path).await.is_ok()
}

async fn is_symlink_async(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path)
        .await
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

async fn is_type_async(path: &Path, kind: FileKind) -> bool {
    let Ok(meta) = tokio::fs::symlink_metadata(path).await else {
        return false;
    };

    #[cfg(unix)]
    {
        let ft = meta.file_type();
        match kind {
            FileKind::Block => ft.is_block_device(),
            FileKind::Char => ft.is_char_device(),
            FileKind::Fifo => ft.is_fifo(),
            FileKind::Socket => ft.is_socket(),
        }
    }

    #[cfg(not(unix))]
    {
        let _ = kind;
        false
    }
}

async fn is_executable_async(path: &Path) -> bool {
    #[cfg(unix)]
    {
        if let Ok(meta) = tokio::fs::metadata(path).await {
            return meta.permissions().mode() & 0o111 != 0;
        }
    }
    false
}

async fn can_open_async(path: &Path, read: bool, write: bool) -> bool {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.read(read).write(write);
    opts.open(path).await.is_ok()
}

async fn can_read_dir_async(path: &Path) -> bool {
    match tokio::fs::read_dir(path).await {
        Ok(mut rd) => rd.next_entry().await.is_ok(),
        Err(_) => false,
    }
}

const WRITE_PROBE_RETRIES: u32 = 4;

async fn can_write_dir_async(path: &Path) -> bool {
    // Directory writability is not reliably inferable from mode bits alone (ACLs, Windows, etc.).
    // We probe by creating (and best-effort removing) a temporary file inside the directory.
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create_new(true);

    for attempt in 0..WRITE_PROBE_RETRIES {
        let probe = path.join(format!(".ward_write_probe_{pid}_{nanos}_{attempt}"));
        match opts.open(&probe).await {
            Ok(f) => {
                drop(f);
                // Best-effort cleanup. If removal fails, the directory was still writable.
                let _ = tokio::fs::remove_file(&probe).await;
                return true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(_) => return false,
        }
    }

    false
}


async fn is_readable_path_async(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(meta) => {
            if meta.is_dir() {
                can_read_dir_async(path).await
            } else {
                can_open_async(path, true, false).await
            }
        }
        Err(_) => false,
    }
}

async fn is_writable_path_async(path: &Path) -> bool {
    match tokio::fs::metadata(path).await {
        Ok(meta) => {
            if meta.is_dir() {
                can_write_dir_async(path).await
            } else {
                // On Unix, opening the write end of a FIFO can block until a reader exists.
                // Avoid probe hangs by treating FIFOs/sockets as not writable for this check.
                #[cfg(unix)]
                {
                    let ft = meta.file_type();
                    if ft.is_fifo() || ft.is_socket() {
                        return false;
                    }
                }
                can_open_async(path, false, true).await
            }
        }
        Err(_) => false,
    }
}


async fn mkdir_async(path: &Path, options: MkdirOpts) -> OpOutcome {
    let target = path.to_path_buf();

    if exists_async(&target).await {
        match tokio::fs::metadata(&target).await {
            Ok(meta) => {
                if meta.is_dir() {
                    return OpOutcome::ok();
                }

                if options.force {
                    // Best-effort: remove the existing non-directory path without following symlinks.
                    maybe_force_remove(&target).await;
                    if exists_async(&target).await {
                        return OpOutcome::fail_msg("path exists and could not be removed");
                    }
                } else {
                    return OpOutcome::fail_msg("path exists and is not a directory");
                }
            }
            Err(e) => {
                if options.force {
                    // `metadata()` follows symlinks; it can fail on dangling symlinks.
                    // If force is set, try removing the path and continue.
                    maybe_force_remove(&target).await;
                    if exists_async(&target).await {
                        return OpOutcome::fail(e);
                    }
                } else {
                    return OpOutcome::fail(e);
                }
            }
        }
    }

    let res = if options.recursive {
        tokio::fs::create_dir_all(&target).await
    } else {
        tokio::fs::create_dir(&target).await
    };

    if let Err(e) = res {
        // Race/TOCTOU: if we lost the race and the path is now a directory, treat as success.
        if e.kind() == std::io::ErrorKind::AlreadyExists
            && tokio::fs::metadata(&target).await.map(|m| m.is_dir()).unwrap_or(false)
        {
            return OpOutcome::ok();
        }
        return OpOutcome::fail(e);
    }

    #[cfg(unix)]
    {
        if let Ok(meta) = tokio::fs::symlink_metadata(&target).await
            && meta.is_dir()
            && let Some(mode) = options.mode
        {
            let perms = std::fs::Permissions::from_mode(mode);
            let _ = tokio::fs::set_permissions(&target, perms).await;
        }
    }

    OpOutcome::ok()
}


async fn rm_async(path: &Path, options: RemoveOpts) -> OpOutcome {
    let target = path.to_path_buf();

    if !exists_async(&target).await {
        return if options.force {
            OpOutcome::ok()
        } else {
            OpOutcome::fail_msg("path not found")
        };
    }

    let meta = match tokio::fs::symlink_metadata(&target).await {
        Ok(m) => m,
        Err(e) => {
            return if options.force {
                OpOutcome::ok()
            } else {
                OpOutcome::fail(e)
            };
        }
    };

    let ft = meta.file_type();
    let is_dir = ft.is_dir();
    let is_symlink = ft.is_symlink();

    let res = if is_dir && !is_symlink {
        if options.recursive {
            tokio::fs::remove_dir_all(&target).await
        } else {
            tokio::fs::remove_dir(&target).await
        }
    } else {
        tokio::fs::remove_file(&target).await
    };

    match res {
        Ok(()) => OpOutcome::ok(),
        Err(e) if options.force && e.kind() == std::io::ErrorKind::NotFound => OpOutcome::ok(),
        Err(e) => OpOutcome::fail(e),
    }
}

async fn unlink_async(path: &Path, options: ForceOnly) -> OpOutcome {
    match tokio::fs::remove_file(path).await {
        Ok(()) => OpOutcome::ok(),
        Err(e) if options.force && e.kind() == std::io::ErrorKind::NotFound => OpOutcome::ok(),
        Err(e) => OpOutcome::fail(e),
    }
}

async fn chmod_async(path: &Path, mode: u32, options: RecursiveForce) -> OpOutcome {
    #[cfg(not(unix))]
    {
        let _ = (path, mode, options);
        return OpOutcome::ok();
    }

    #[cfg(unix)]
    {
        let target = path.to_path_buf();
        let mut success = true;
        let mut first_err: Option<String> = None;
        let perms = std::fs::Permissions::from_mode(mode);

        if options.recursive {
            let mut queue = VecDeque::from([target]);
            while let Some(current) = queue.pop_front() {
                if let Ok(meta) = tokio::fs::symlink_metadata(&current).await
                    && meta.is_dir()
                    && let Ok(mut rd) = tokio::fs::read_dir(&current).await
                {
                    while let Ok(Some(ent)) = rd.next_entry().await {
                        let p = ent.path();
                        let ft = ent.file_type().await.ok();
                        if let Some(ft) = ft {
                            if ft.is_symlink() {
                                continue;
                            }
                            if ft.is_dir() {
                                queue.push_back(p.clone());
                            }
                        }

                        if let Err(e) = tokio::fs::set_permissions(&p, perms.clone()).await {
                            success = false;
                            first_err.get_or_insert_with(|| e.to_string());
                            if !options.force {
                                return OpOutcome::fail(e);
                            }
                        }
                    }
                }

                // Avoid mutating symlink targets in recursive chmod.
                if let Ok(meta) = tokio::fs::symlink_metadata(&current).await
                    && meta.file_type().is_symlink()
                {
                    continue;
                }

                if let Err(e) = tokio::fs::set_permissions(&current, perms.clone()).await {
                    success = false;
                    first_err.get_or_insert_with(|| e.to_string());
                    if !options.force {
                        return OpOutcome::fail(e);
                    }
                }
            }
        } else {
            if let Ok(meta) = tokio::fs::symlink_metadata(&target).await
                && meta.file_type().is_symlink()
            {
                // WARN: For safety, we do not follow symlinks.
                return if options.force {
                    OpOutcome::ok()
                } else {
                    OpOutcome::fail_msg("refusing to chmod symlink")
                };
            }

            if let Err(e) = tokio::fs::set_permissions(&target, perms).await {
                success = false;
                first_err.get_or_insert_with(|| e.to_string());
                if !options.force {
                    return OpOutcome::fail(e);
                }
            }
        }

        if success {
            OpOutcome::ok()
        } else {
            OpOutcome {
                ok: false,
                err: first_err,
            }
        }
    }
}

async fn chown_async(path: &Path, uid: u32, gid: u32, options: RecursiveForce) -> OpOutcome {
    #[cfg(not(unix))]
    {
        let _ = (path, uid, gid, options);
        return OpOutcome::ok();
    }

    #[cfg(unix)]
    {
        let target = path.to_path_buf();
        let mut success = true;
        let mut first_err: Option<String> = None;

        let owner = Some(Uid::from_raw(uid));
        let group = Some(Gid::from_raw(gid));
        let apply = |p: &Path| nix_chown(p, owner, group);

        if options.recursive {
            let mut queue = VecDeque::from([target]);
            while let Some(current) = queue.pop_front() {
                if let Ok(meta) = tokio::fs::symlink_metadata(&current).await
                    && meta.is_dir()
                    && let Ok(mut rd) = tokio::fs::read_dir(&current).await
                {
                    while let Ok(Some(ent)) = rd.next_entry().await {
                        let p = ent.path();
                        let ft = ent.file_type().await.ok();
                        if let Some(ft) = ft {
                            if ft.is_symlink() {
                                continue;
                            }
                            if ft.is_dir() {
                                queue.push_back(p.clone());
                            }
                        }

                        if let Err(e) = apply(&p) {
                            success = false;
                            first_err.get_or_insert_with(|| e.to_string());
                            if !options.force {
                                return OpOutcome::fail(e);
                            }
                        }
                    }
                }

                // Avoid mutating symlink targets in recursive chown.
                if let Ok(meta) = tokio::fs::symlink_metadata(&current).await
                    && meta.file_type().is_symlink()
                {
                    continue;
                }

                if let Err(e) = apply(&current) {
                    success = false;
                    first_err.get_or_insert_with(|| e.to_string());
                    if !options.force {
                        return OpOutcome::fail(e);
                    }
                }
            }
        } else if let Ok(meta) = tokio::fs::symlink_metadata(&target).await
            && meta.file_type().is_symlink()
        {
            // WARN: For safety, we do not follow symlinks.
            return if options.force {
                OpOutcome::ok()
            } else {
                OpOutcome::fail_msg("refusing to chown symlink")
            };
        } else if let Err(e) = apply(&target) {
            success = false;
            first_err.get_or_insert_with(|| e.to_string());
            if !options.force {
                return OpOutcome::fail(e);
            }
        }

        if success {
            OpOutcome::ok()
        } else {
            OpOutcome {
                ok: false,
                err: first_err,
            }
        }
    }
}

async fn maybe_force_remove(dest: &Path) {
    // WARN: Avoid following symlinks during force removal.
    if let Ok(meta) = tokio::fs::symlink_metadata(dest).await {
        if meta.file_type().is_symlink() || meta.is_file() {
            let _ = tokio::fs::remove_file(dest).await;
            return;
        }
        if meta.is_dir() {
            let _ = tokio::fs::remove_dir_all(dest).await;
            return;
        }
    }

    let _ = tokio::fs::remove_file(dest).await;
    let _ = tokio::fs::remove_dir_all(dest).await;
}

async fn rename_async(old_path: &Path, new_path: &Path, options: ForceOnly) -> OpOutcome {
    if options.force && exists_async(new_path).await {
        maybe_force_remove(new_path).await;
    }
    match tokio::fs::rename(old_path, new_path).await {
        Ok(()) => OpOutcome::ok(),
        Err(e) => OpOutcome::fail(e),
    }
}

fn is_cross_device_error(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        // EXDEV (cross-device link) is 18 on POSIX systems.
        e.raw_os_error() == Some(18)
    }
    #[cfg(target_os = "windows")]
    {
        // ERROR_NOT_SAME_DEVICE is 17.
        return e.raw_os_error() == Some(17);
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    false
}

async fn move_async(from: &Path, to: &Path, options: ForceOnly) -> OpOutcome {
    if options.force && exists_async(to).await {
        maybe_force_remove(to).await;
    }

    match tokio::fs::rename(from, to).await {
        Ok(()) => OpOutcome::ok(),
        Err(e) => {
            if !is_cross_device_error(&e) {
                return OpOutcome::fail(e);
            }

            // Cross-device fallback for regular files only:
            // copy + remove. This intentionally does not support dirs or symlinks yet.
            let meta = match tokio::fs::symlink_metadata(from).await {
                Ok(m) => m,
                Err(me) => return OpOutcome::fail(me),
            };

            let ft = meta.file_type();
            if ft.is_symlink() {
                return OpOutcome::fail_msg("cross-device move of symlinks is not supported");
            }
            if meta.is_dir() {
                return OpOutcome::fail_msg("cross-device move of directories is not supported");
            }

            if let Err(ce) = tokio::fs::copy(from, to).await {
                return OpOutcome::fail(ce);
            }
            if let Err(re) = tokio::fs::remove_file(from).await {
                return OpOutcome::fail(re);
            }
            OpOutcome::ok()
        }
    }
}


async fn link_async(old_path: &Path, new_path: &Path, options: ForceOnly) -> OpOutcome {
    if options.force && exists_async(new_path).await {
        maybe_force_remove(new_path).await;
    }
    match tokio::fs::hard_link(old_path, new_path).await {
        Ok(()) => OpOutcome::ok(),
        Err(e) => OpOutcome::fail(e),
    }
}

async fn symlink_path_async(old_path: &Path, new_path: &Path, options: ForceOnly) -> OpOutcome {
    let source = old_path.to_path_buf();
    let dest = new_path.to_path_buf();

    if options.force && exists_async(&dest).await {
        maybe_force_remove(&dest).await;
    }

    #[cfg(any(unix, target_os = "windows"))]
    {
        let res = tokio::task::spawn_blocking(move || symlink_platform(&source, &dest)).await;
        match res {
            Ok(Ok(())) => OpOutcome::ok(),
            Ok(Err(e)) => OpOutcome::fail(e),
            Err(e) => OpOutcome::fail(e),
        }
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = (source, dest);
        return if options.force {
            OpOutcome::ok()
        } else {
            OpOutcome::fail_msg("symlink is not supported on this platform")
        };
    }
}

async fn readlink_async(path: &Path) -> mlua::Result<Option<String>> {
    match tokio::fs::read_link(path).await {
        Ok(p) => Ok(Some(p.to_string_lossy().into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        // Not a symlink (platform-specific error kind); return nil per API docs.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(None),
        Err(e) => Err(mlua::Error::external(e)),
    }
}



async fn realpath_async(path: &Path) -> mlua::Result<Option<String>> {
    match tokio::fs::canonicalize(path).await {
        Ok(p) => Ok(Some(p.to_string_lossy().into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(mlua::Error::external(e)),
    }
}



fn dirname(path: &Path) -> String {
    let path = PathBuf::from(path);
    path.parent()
        .map_or_else(|| String::from("."), |p| p.to_string_lossy().into_owned())
}

fn basename(path: &Path) -> String {
    let path = PathBuf::from(path);
    path.file_name()
        .map_or_else(|| OsString::from(""), OsString::from)
        .to_string_lossy()
        .into_owned()
}

async fn touch_async(path: &Path, options: TouchOpts) -> OpOutcome {
    let target = path.to_path_buf();

    if options.recursive
        && let Some(parent) = target.parent()
    {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    if !exists_async(&target).await {
        match tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&target)
            .await
        {
            Ok(_) => {}
            Err(e) => return OpOutcome::fail(e),
        }
    }

    // filetime is sync; run in blocking pool
    let res = tokio::task::spawn_blocking(move || {
        let now = FileTime::from_system_time(SystemTime::now());
        filetime::set_file_times(&target, now, now)
    })
    .await;

    match res {
        Ok(Ok(())) => OpOutcome::ok(),
        Ok(Err(e)) => OpOutcome::fail(e),
        Err(e) => OpOutcome::fail(e),
    }
}

async fn read_async(path: &Path, options: ReadOpts) -> mlua::Result<Vec<u8>> {
    let bytes = tokio::fs::read(path).await.map_err(mlua::Error::external)?;
    if options.mode == ReadMode::Text {
        std::str::from_utf8(&bytes).map_err(mlua::Error::external)?;
    }
    Ok(bytes)
}

async fn write_async(path: &Path, bytes: Vec<u8>, options: WriteOpts) -> OpOutcome {
    match options.mode {
        WriteMode::Overwrite => {
            let open_res = tokio::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .create(true)
                .open(path)
                .await;

            let mut f = match open_res {
                Ok(f) => f,
                Err(e) => return OpOutcome::fail(e),
            };

            match f.write_all(&bytes).await {
                Ok(()) => OpOutcome::ok(),
                Err(e) => OpOutcome::fail(e),
            }
        }
        WriteMode::Append => {
            let open_res = tokio::fs::OpenOptions::new()
                .write(true)
                .append(true)
                .create(true)
                .open(path)
                .await;

            let mut f = match open_res {
                Ok(f) => f,
                Err(e) => return OpOutcome::fail(e),
            };

            match f.write_all(&bytes).await {
                Ok(()) => OpOutcome::ok(),
                Err(e) => OpOutcome::fail(e),
            }
        }
        WriteMode::Prepend => {
            let existing = if exists_async(path).await {
                match tokio::fs::read(path).await {
                    Ok(v) => v,
                    Err(e) => return OpOutcome::fail(e),
                }
            } else {
                Vec::new()
            };

            let open_res = tokio::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .create(true)
                .open(path)
                .await;

            let mut f = match open_res {
                Ok(f) => f,
                Err(e) => return OpOutcome::fail(e),
            };

            if let Err(e) = f.write_all(&bytes).await {
                return OpOutcome::fail(e);
            }
            if let Err(e) = f.write_all(&existing).await {
                return OpOutcome::fail(e);
            }
            OpOutcome::ok()
        }
    }
}

async fn list_async(path: &Path, options: ListOpts) -> mlua::Result<Vec<String>> {
    let root = path.to_path_buf();
    let mut entries = Vec::new();

    if !exists_async(&root).await {
        return Ok(entries);
    }

    if options.recursive {
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root, 0)]);
        while let Some((dir, depth)) = queue.pop_front() {
            if options.depth > 0 && depth > options.depth {
                continue;
            }

            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(ent)) = rd.next_entry().await {
                let p = ent.path();
                let ft = ent.file_type().await.ok();
                let is_dir = ft.as_ref().is_some_and(std::fs::FileType::is_dir);
                let is_symlink = ft.as_ref().is_some_and(std::fs::FileType::is_symlink);

                // Depth is measured from the root dir's *children* as depth=1.
                // When a max depth is set, we must not include entries beyond it.
                let next_depth = depth.saturating_add(1);
                let within_depth = options.depth == 0 || next_depth <= options.depth;

                if is_dir && !is_symlink && within_depth {
                    queue.push_back((p.clone(), next_depth));
                }
                if within_depth && should_include(&p, is_dir, &options) {
                    entries.push(p.to_string_lossy().into_owned());
                }
            }
        }
    } else {
        let Ok(mut rd) = tokio::fs::read_dir(&root).await else {
            return Ok(entries);
        };
        while let Ok(Some(ent)) = rd.next_entry().await {
            let p = ent.path();
            let is_dir = ent.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            if should_include(&p, is_dir, &options) {
                entries.push(p.to_string_lossy().into_owned());
            }
        }
    }

    Ok(entries)
}

async fn copy_async(from: &Path, to: &Path, options: ForceOnly) -> OpOutcome {
    if options.force && exists_async(to).await {
        maybe_force_remove(to).await;
    }
    match tokio::fs::copy(from, to).await {
        Ok(_) => OpOutcome::ok(),
        Err(e) => OpOutcome::fail(e),
    }
}

fn join(path: PathBuf, rest: mlua::Variadic<Value>) -> mlua::Result<String> {
    let mut buf = path;
    for part in rest.iter() {
        let addition = value_to_path_buf(part.clone())?;
        if addition.is_absolute() {
            buf = addition;
            continue;
        }
        buf.push(addition);
    }
    Ok(buf.to_string_lossy().into_owned())
}

fn glob_paths(pattern: String) -> Result<Vec<String>, String> {
    let mut matches = Vec::new();

    let iter = glob(&pattern).map_err(|e| e.to_string())?;
    for entry in iter {
        match entry {
            Ok(path) => matches.push(path.to_string_lossy().into_owned()),
            Err(err) => return Err(err.to_string()),
        }
    }

    Ok(matches)
}

async fn tempdir_async(prefix: Option<String>) -> mlua::Result<String> {
    let prefix = prefix.unwrap_or_else(|| String::from("tmp"));
    let mut counter: u64 = 0;
    let base = std::env::temp_dir();

    loop {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let candidate = base.join(format!("{prefix}-{now}-{counter}"));
        counter = counter.wrapping_add(1);

        if !exists_async(&candidate).await {
            match tokio::fs::create_dir(&candidate).await {
                Ok(()) => return Ok(candidate.to_string_lossy().into_owned()),
                Err(e) => {
                    if counter > 10_000 {
                        return Err(mlua::Error::external(e));
                    }
                }
            }
        }
    }
}

fn should_include(path: &Path, is_dir: bool, opts: &ListOpts) -> bool {
    let include_dirs = opts.dirs || !opts.files;
    let include_files = opts.files || !opts.dirs;
    let include = if is_dir { include_dirs } else { include_files };

    if !include {
        return false;
    }
    if let Some(regex) = &opts.regex {
        return regex.is_match(&path.to_string_lossy());
    }
    true
}

fn value_to_path_buf(value: Value) -> mlua::Result<PathBuf> {
    match value {
        Value::String(s) => Ok(PathBuf::from(s.to_str()?.to_owned())),
        Value::UserData(u) => {
            if let Ok(p) = u.borrow::<path::PathObj>() {
                Ok(p.path.clone())
            } else {
                Err(mlua::Error::external("expected path or string"))
            }
        }
        other => Err(mlua::Error::external(format!("expected path or string, got {other:?}"))),
    }
}

fn value_to_string(value: mlua::Value) -> mlua::Result<String> {
    match value {
        mlua::Value::String(s) => Ok(s.to_string_lossy()),
        mlua::Value::Nil => Ok(String::new()),
        mlua::Value::Integer(i) => Ok(i.to_string()),
        mlua::Value::Number(n) => Ok(n.to_string()),
        mlua::Value::Boolean(b) => Ok(b.to_string()),
        other => Ok(format!("{other:?}")),
    }
}

fn value_to_bytes(value: mlua::Value) -> mlua::Result<Vec<u8>> {
    match value {
        mlua::Value::String(s) => Ok(s.as_bytes().to_vec()),
        mlua::Value::Nil => Ok(Vec::new()),
        mlua::Value::Integer(i) => Ok(i.to_string().into_bytes()),
        mlua::Value::Number(n) => Ok(n.to_string().into_bytes()),
        mlua::Value::Boolean(b) => Ok(b.to_string().into_bytes()),
        other => Ok(format!("{other:?}").into_bytes()),
    }
}

#[derive(Default)]
struct MkdirOpts {
    recursive: bool,
    mode: Option<u32>,
    force: bool,
}
impl MkdirOpts {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            Ok(Self {
                recursive: table.get::<Option<bool>>("recursive")?.unwrap_or(false),
                mode: table.get::<Option<u32>>("mode")?.or(Some(0o755)),
                force: table.get::<Option<bool>>("force")?.unwrap_or(false),
            })
        } else {
            Ok(Self {
                recursive: false,
                mode: Some(0o755),
                force: false,
            })
        }
    }
}

#[derive(Default)]
struct RemoveOpts {
    recursive: bool,
    force: bool,
}
impl RemoveOpts {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            Ok(Self {
                recursive: table.get::<Option<bool>>("recursive")?.unwrap_or(false),
                force: table.get::<Option<bool>>("force")?.unwrap_or(false),
            })
        } else {
            Ok(Self::default())
        }
    }
}

#[derive(Default)]
struct RecursiveForce {
    recursive: bool,
    force: bool,
}
impl RecursiveForce {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            Ok(Self {
                recursive: table.get::<Option<bool>>("recursive")?.unwrap_or(false),
                force: table.get::<Option<bool>>("force")?.unwrap_or(false),
            })
        } else {
            Ok(Self::default())
        }
    }
}

#[derive(Default)]
struct ForceOnly {
    force: bool,
}
impl ForceOnly {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            Ok(Self {
                force: table.get::<Option<bool>>("force")?.unwrap_or(false),
            })
        } else {
            Ok(Self { force: false })
        }
    }
}

#[derive(Default)]
struct TouchOpts {
    recursive: bool,
}
impl TouchOpts {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            Ok(Self {
                recursive: table.get::<Option<bool>>("recursive")?.unwrap_or(false),
            })
        } else {
            Ok(Self::default())
        }
    }
}

#[derive(Default)]
struct ReadOpts {
    mode: ReadMode,
}
#[derive(Default, PartialEq)]
enum ReadMode {
    #[default]
    Text,
    Binary,
}
impl ReadOpts {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            let mode: Option<String> = table.get::<Option<String>>("mode").unwrap_or(None);
            let mode = match mode.as_deref() {
                Some("binary") => ReadMode::Binary,
                _ => ReadMode::Text,
            };
            Ok(Self { mode })
        } else {
            Ok(Self::default())
        }
    }
}

#[derive(Default)]
struct WriteOpts {
    mode: WriteMode,
    binary: bool,
}
#[derive(Default, PartialEq)]
enum WriteMode {
    #[default]
    Overwrite,
    Append,
    Prepend,
}
impl WriteOpts {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            let mut opts = Self {
                mode: WriteMode::Overwrite,
                binary: table.get::<Option<bool>>("binary")?.unwrap_or(false),
            };

            if let Ok(append) = table.get::<bool>("append")
                && append
            {
                opts.mode = WriteMode::Append;
            }

            if let Ok(mode) = table.get::<Option<String>>("mode")
                && let Some(mode) = mode
            {
                opts.mode = match mode.to_lowercase().as_str() {
                    "append" => WriteMode::Append,
                    "prepend" => WriteMode::Prepend,
                    "binary" => {
                        opts.binary = true;
                        WriteMode::Overwrite
                    }
                    _ => WriteMode::Overwrite,
                };
                if mode.eq_ignore_ascii_case("binary") {
                    opts.binary = true;
                }
            }

            if let Ok(mode) = table.get::<Option<String>>("data_mode")
                && let Some(mode) = mode
                && mode.eq_ignore_ascii_case("binary")
            {
                opts.binary = true;
            }

            Ok(opts)
        } else {
            Ok(Self {
                mode: WriteMode::Overwrite,
                binary: false,
            })
        }
    }
}

struct ListOpts {
    dirs: bool,
    files: bool,
    recursive: bool,
    depth: usize,
    regex: Option<regex::Regex>,
}
impl ListOpts {
    fn from_value(value: mlua::Value) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            let regex_string: Option<String> = table.get::<Option<String>>("regex")?;
            let regex = if let Some(regex_string) = regex_string {
                Some(regex::Regex::new(&regex_string).map_err(mlua::Error::external)?)
            } else {
                None
            };
            Ok(Self {
                dirs: table.get::<Option<bool>>("dirs")?.unwrap_or(false),
                files: table.get::<Option<bool>>("files")?.unwrap_or(false),
                recursive: table.get::<Option<bool>>("recursive")?.unwrap_or(false),
                depth: table.get::<Option<usize>>("depth")?.unwrap_or(0),
                regex,
            })
        } else {
            Ok(Self {
                dirs: false,
                files: false,
                recursive: false,
                depth: 0,
                regex: None,
            })
        }
    }
}
