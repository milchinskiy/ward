#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unnecessary_wraps)]

use std::io::Read;

use mlua::{Lua, Table};
use tokio::io::AsyncReadExt;

/// Initializes the `crypto` module.
///
/// Notes:
/// - `*_file` variants stream files and are async.
/// - Byte variants expect a Lua string (binary-safe).
///
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    // Bytes
    t.set(
        "sha256",
        lua.create_function(|_, bytes: mlua::String| Ok(sha256_bytes(bytes.as_bytes())))?,
    )?;
    t.set(
        "sha1",
        lua.create_function(|_, bytes: mlua::String| Ok(sha1_bytes(bytes.as_bytes())))?,
    )?;
    t.set(
        "md5",
        lua.create_function(|_, bytes: mlua::String| Ok(md5_bytes(bytes.as_bytes())))?,
    )?;

    // Files (streamed)
    t.set(
        "sha256_file",
        lua.create_async_function(|_, path: String| async move { sha256_file(std::path::PathBuf::from(path)).await })?,
    )?;
    t.set(
        "sha1_file",
        lua.create_async_function(|_, path: String| async move { sha1_file(std::path::PathBuf::from(path)).await })?,
    )?;
    t.set(
        "md5_file",
        lua.create_async_function(|_, path: String| async move { md5_file(std::path::PathBuf::from(path)).await })?,
    )?;

    Ok(t)
}

fn to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

fn sha256_bytes(data: impl AsRef<[u8]>) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(data);
    to_hex(&h.finalize())
}

fn sha1_bytes(data: impl AsRef<[u8]>) -> String {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    h.update(data);
    to_hex(&h.finalize())
}

fn md5_bytes(data: impl AsRef<[u8]>) -> String {
    let digest = md5::compute(data);
    format!("{digest:x}")
}

async fn sha256_file(path: std::path::PathBuf) -> mlua::Result<String> {
    use sha2::Digest;
    let mut f = tokio::fs::File::open(&path).await.map_err(mlua::Error::external)?;

    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024].into_boxed_slice();
    loop {
        let n = f.read(&mut buf).await.map_err(mlua::Error::external)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

async fn sha1_file(path: std::path::PathBuf) -> mlua::Result<String> {
    use sha1::Digest;
    let mut f = tokio::fs::File::open(&path).await.map_err(mlua::Error::external)?;

    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 64 * 1024].into_boxed_slice();
    loop {
        let n = f.read(&mut buf).await.map_err(mlua::Error::external)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

async fn md5_file(path: std::path::PathBuf) -> mlua::Result<String> {
    // md5 crate is sync; stream on a blocking thread.
    tokio::task::spawn_blocking(move || {
        let mut f = std::fs::File::open(&path)?;
        let mut ctx = md5::Context::new();
        let mut buf = vec![0u8; 64 * 1024].into_boxed_slice();
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            ctx.consume(&buf[..n]);
        }
        let digest = ctx.finalize();
        Ok::<String, std::io::Error>(format!("{digest:x}"))
    })
    .await
    .map_err(mlua::Error::external)?
    .map_err(mlua::Error::external)
}
