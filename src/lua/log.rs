use rustlog::HumanDuration;
use std::time::Instant;

/// Initializes the `log` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let log = lua.create_table()?;

    let rlog = std::sync::Arc::new(
        rustlog::local::Logger::builder()
            .set_level(rustlog::Level::Trace)
            .set_show_thread_id(false)
            .set_show_time(true)
            .set_show_group(true)
            .set_show_file_line(false)
            .set_color_mode(rustlog::ColorMode::Auto)
            .build()?,
    );

    for level in ["trace", "debug", "info", "warn", "error", "fatal"] {
        let rlog_clone = rlog.clone();
        log.set(
            level,
            lua.create_function(move |_, args: mlua::Variadic<mlua::Value>| {
                let args = variadic_to_string(args);
                match level {
                    "trace" => rustlog::local::trace!(&rlog_clone, "{}", args),
                    "debug" => rustlog::local::debug!(&rlog_clone, "{}", args),
                    "info" => rustlog::local::info!(&rlog_clone, "{}", args),
                    "warn" => rustlog::local::warn!(&rlog_clone, "{}", args),
                    "error" => rustlog::local::error!(&rlog_clone, "{}", args),
                    "fatal" => rustlog::local::fatal!(&rlog_clone, "{}", args),
                    _ => unreachable!(),
                }
                Ok(())
            })?,
        )?;
    }

    for level in ["ltrace", "ldebug", "linfo", "lwarn", "lerror", "lfatal"] {
        let rlog_clone = rlog.clone();
        log.set(
            level,
            lua.create_function(move |_, (label, args): (mlua::BorrowedStr, mlua::Variadic<mlua::Value>)| {
                let lbl = label.trim();
                let args = variadic_to_string(args);
                match level {
                    "ltrace" => rustlog::local::trace!(&rlog_clone, "[{}] {}", lbl, args),
                    "ldebug" => rustlog::local::debug!(&rlog_clone, "[{}] {}", lbl, args),
                    "linfo" => rustlog::local::info!(&rlog_clone, "[{}] {}", lbl, args),
                    "lwarn" => rustlog::local::warn!(&rlog_clone, "[{}] {}", lbl, args),
                    "lerror" => rustlog::local::error!(&rlog_clone, "[{}] {}", lbl, args),
                    "lfatal" => rustlog::local::fatal!(&rlog_clone, "[{}] {}", lbl, args),
                    _ => unreachable!(),
                }
                Ok(())
            })?,
        )?;
    }

    {
        let rlog_clone = rlog.clone();
        log.set(
            "set_level",
            lua.create_function(move |_, level: mlua::String| {
                let level = match level.to_str()?.to_ascii_lowercase().as_str() {
                    "trace" => rustlog::Level::Trace,
                    "debug" => rustlog::Level::Debug,
                    "info" => rustlog::Level::Info,
                    "warn" | "warning" => rustlog::Level::Warn,
                    "error" => rustlog::Level::Error,
                    "fatal" => rustlog::Level::Fatal,
                    _ => return Err(mlua::Error::RuntimeError("invalid log level".into())),
                };
                rlog_clone.set_level(level);
                Ok(())
            })?,
        )?;
    }

    {
        let rlog_clone = rlog;
        log.set(
            "time",
            lua.create_function(move |_, (label, func): (mlua::String, mlua::Function)| {
                let start = Instant::now();
                let result = func.call::<mlua::Value>(())?;
                let duration = HumanDuration(start.elapsed());
                rustlog::local::info!(&rlog_clone, "[{}] took {}", label.to_str()?, duration);
                Ok(result)
            })?,
        )?;
    }

    Ok(log)
}

fn variadic_to_string(args: mlua::Variadic<mlua::Value>) -> String {
    let mut s: Vec<String> = Vec::with_capacity(args.len());
    for arg in args {
        s.push(match arg {
            mlua::Value::String(s) => s.to_string_lossy(),
            mlua::Value::Nil
            | mlua::Value::Boolean(_)
            | mlua::Value::Integer(_)
            | mlua::Value::Number(_)
            | mlua::Value::Table(_) => serde_json::to_string_pretty(&arg).unwrap_or_else(|_| "<unknown>".into()),
            mlua::Value::UserData(_) => "<userdata>".into(),
            mlua::Value::Other(_) => "<other>".into(),
            mlua::Value::Function(f) => format!("<function {:?}>", f.info().name),
            mlua::Value::Thread(t) => format!("<thread {t:?}>"),
            mlua::Value::Error(e) => format!("<error {e:?}>"),
            mlua::Value::LightUserData(u) => format!("<lightuserdata {u:?}>"),
        });
    }
    s.join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn variadic_values_render_as_strings() {
        let lua = mlua::Lua::new();
        let table = lua.create_table_from([(1, 2)]).unwrap();
        let rendered = variadic_to_string(mlua::Variadic::from_iter([
            mlua::Value::String(lua.create_string("hello").unwrap()),
            mlua::Value::Integer(42),
            mlua::Value::Table(table),
        ]));

        assert!(rendered.contains("hello"));
        assert!(rendered.contains("42"));
        assert!(rendered.contains('2'));
    }
}
