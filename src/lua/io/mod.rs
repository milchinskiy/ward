use std::sync::Arc;

use mlua::{Lua, Table, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Initializes the `io` module
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    // Shared handles (serialized with async Mutex to prevent concurrent reads/writes)
    let stdin = Arc::new(Mutex::new(BufReader::new(io::stdin())));
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let stderr = Arc::new(Mutex::new(io::stderr()));

    // read_all(): async -> String
    table.set(
        "read_all",
        lua.create_async_function({
            let stdin = Arc::clone(&stdin);
            move |_lua, ()| {
                let stdin = Arc::clone(&stdin);
                async move {
                    let mut guard = stdin.lock().await;
                    let mut buffer = String::new();
                    guard.read_to_string(&mut buffer).await.map_err(mlua::Error::external)?;
                    drop(guard);
                    Ok(buffer)
                }
            }
        })?,
    )?;

    // read_line(): async -> string|nil
    // Returns nil on EOF. Strips trailing "\n" and optional "\r".
    table.set(
        "read_line",
        lua.create_async_function({
            let stdin = Arc::clone(&stdin);
            move |_lua, ()| {
                let stdin = Arc::clone(&stdin);
                async move {
                    let mut guard = stdin.lock().await;

                    let mut line = String::new();
                    let bytes = guard.read_line(&mut line).await.map_err(mlua::Error::external)?;
                    drop(guard);

                    if bytes == 0 {
                        return Ok::<Option<String>, mlua::Error>(None);
                    }

                    if line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }

                    Ok(Some(line))
                }
            }
        })?,
    )?;

    // read_lines(): sync -> function; the returned function is async and yields string|nil per call.
    table.set(
        "read_lines",
        lua.create_function({
            let stdin = Arc::clone(&stdin);
            move |lua, ()| read_lines(lua, Arc::clone(&stdin))
        })?,
    )?;

    // write_stdout(text): async -> bool
    table.set(
        "write_stdout",
        lua.create_async_function({
            let stdout = Arc::clone(&stdout);
            move |_lua, text: String| {
                let stdout = Arc::clone(&stdout);
                async move {
                    let mut out = stdout.lock().await;
                    out.write_all(text.as_bytes()).await.map_err(mlua::Error::external)?;
                    drop(out);
                    Ok(true)
                }
            }
        })?,
    )?;

    // write_stderr(text): async -> bool
    table.set(
        "write_stderr",
        lua.create_async_function({
            let stderr = Arc::clone(&stderr);
            move |_lua, text: String| {
                let stderr = Arc::clone(&stderr);
                async move {
                    let mut err = stderr.lock().await;
                    err.write_all(text.as_bytes()).await.map_err(mlua::Error::external)?;
                    drop(err);
                    Ok(true)
                }
            }
        })?,
    )?;

    // flush_stdout(): async -> ()
    table.set(
        "flush_stdout",
        lua.create_async_function({
            let stdout = Arc::clone(&stdout);
            move |_lua, ()| {
                let stdout = Arc::clone(&stdout);
                async move {
                    let mut out = stdout.lock().await;
                    out.flush().await.map_err(mlua::Error::external)?;
                    drop(out);
                    Ok(())
                }
            }
        })?,
    )?;

    // flush_stderr(): async -> ()
    table.set(
        "flush_stderr",
        lua.create_async_function({
            let stderr = Arc::clone(&stderr);
            move |_lua, ()| {
                let stderr = Arc::clone(&stderr);
                async move {
                    let mut err = stderr.lock().await;
                    err.flush().await.map_err(mlua::Error::external)?;
                    drop(err);
                    Ok(())
                }
            }
        })?,
    )?;

    Ok(table)
}

fn read_lines(lua: &Lua, stdin: Arc<Mutex<BufReader<io::Stdin>>>) -> mlua::Result<Value> {
    // This returned function is async; each call reads one line from shared stdin.
    let iter = lua.create_async_function(move |_lua, ()| {
        let stdin = Arc::clone(&stdin);
        async move {
            let mut guard = stdin.lock().await;
            let mut line = String::new();
            let bytes = guard.read_line(&mut line).await.map_err(mlua::Error::external)?;
            drop(guard);

            if bytes == 0 {
                return Ok::<Option<String>, mlua::Error>(None);
            }

            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }

            Ok(Some(line))
        }
    })?;

    Ok(Value::Function(iter))
}
