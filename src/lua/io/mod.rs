use std::sync::Arc;

use mlua::{Lua, Table, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

/// Initializes the `io` module
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    let console = super::console::console(lua);

    // read_all(opts?): async -> bytes string
    // opts can be:
    //   - nil / omitted: unlimited
    //   - number/integer: max_bytes
    //   - table: { max_bytes = number|integer }
    table.set(
        "read_all",
        lua.create_async_function({
            let console = console.clone();
            move |_lua, opts: Option<Value>| {
                let console = console.clone();
                async move {
                    let max_bytes = parse_max_bytes(opts.unwrap_or(Value::Nil))?;
                    let mut guard = console.stdin.lock().await;
                    let mut buf: Vec<u8> = Vec::new();
                    if let Some(max) = max_bytes {
                        // Read up to max+1 so we can detect overflow without allocating unbounded memory.
                        let mut limited = (&mut *guard).take(max.saturating_add(1));
                        limited.read_to_end(&mut buf).await.map_err(mlua::Error::external)?;
                        if (buf.len() as u64) > max {
                            return Err(mlua::Error::external(format!("stdin exceeds max_bytes ({max})")));
                        }
                    } else {
                        guard.read_to_end(&mut buf).await.map_err(mlua::Error::external)?;
                    }
                    drop(guard);
                    Ok(buf)
                }
            }
        })?,
    )?;

    // read_line(): async -> bytes string|nil
    // Returns nil on EOF. Strips trailing "\n" and optional "\r".
    table.set(
        "read_line",
        lua.create_async_function({
            let console = console.clone();
            move |_lua, ()| {
                let console = console.clone();
                async move {
                    let mut guard = console.stdin.lock().await;
                    let mut line: Vec<u8> = Vec::new();
                    let bytes = guard
                        .read_until(b'\n', &mut line)
                        .await
                        .map_err(mlua::Error::external)?;
                    drop(guard);

                    if bytes == 0 {
                        return Ok::<Option<Vec<u8>>, mlua::Error>(None);
                    }

                    if line.ends_with(b"\n") {
                        line.pop();
                        if line.ends_with(b"\r") {
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
            let console = console.clone();
            move |lua, ()| read_lines(lua, console.clone())
        })?,
    )?;

    // write_stdout(text): async -> bool
    table.set(
        "write_stdout",
        lua.create_async_function({
            let console = console.clone();
            move |_lua, text: mlua::String| {
                let console = console.clone();
                async move {
                    let mut out = console.stdout.lock().await;
                    out.write_all(&text.as_bytes()).await.map_err(mlua::Error::external)?;
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
            let console = console.clone();
            move |_lua, text: mlua::String| {
                let console = console.clone();
                async move {
                    let mut err = console.stderr.lock().await;
                    err.write_all(&text.as_bytes()).await.map_err(mlua::Error::external)?;
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
            let console = console.clone();
            move |_lua, ()| {
                let console = console.clone();
                async move {
                    let mut out = console.stdout.lock().await;
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
            move |_lua, ()| {
                let console = console.clone();
                async move {
                    let mut err = console.stderr.lock().await;
                    err.flush().await.map_err(mlua::Error::external)?;
                    drop(err);
                    Ok(())
                }
            }
        })?,
    )?;

    Ok(table)
}

fn read_lines(lua: &Lua, console: Arc<super::console::Console>) -> mlua::Result<Value> {
    // This returned function is async; each call reads one line from shared stdin.
    let iter = lua.create_async_function(move |_lua, ()| {
        let console = Arc::clone(&console);
        async move {
            let mut guard = console.stdin.lock().await;
            let mut line: Vec<u8> = Vec::new();
            let bytes = guard
                .read_until(b'\n', &mut line)
                .await
                .map_err(mlua::Error::external)?;
            drop(guard);

            if bytes == 0 {
                return Ok::<Option<Vec<u8>>, mlua::Error>(None);
            }

            if line.ends_with(b"\n") {
                line.pop();
                if line.ends_with(b"\r") {
                    line.pop();
                }
            }

            Ok(Some(line))
        }
    })?;

    Ok(Value::Function(iter))
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn parse_max_bytes(opts: Value) -> mlua::Result<Option<u64>> {
    match opts {
        Value::Nil => Ok(None),
        Value::Integer(i) => Ok((i > 0).then_some(i as u64)),
        Value::Number(n) => Ok((n.is_finite() && n > 0.0).then_some(n as u64)),
        Value::Table(t) => {
            let v = t.get::<Option<Value>>("max_bytes")?;
            match v {
                None | Some(Value::Nil) => Ok(None),
                Some(Value::Integer(i)) => Ok((i > 0).then_some(i as u64)),
                Some(Value::Number(n)) => Ok((n.is_finite() && n > 0.0).then_some(n as u64)),
                Some(other) => Err(mlua::Error::external(format!("max_bytes must be number, got {other:?}"))),
            }
        }
        other => Err(mlua::Error::external(format!(
            "read_all(opts) expects nil, number, or table, got {other:?}"
        ))),
    }
}
