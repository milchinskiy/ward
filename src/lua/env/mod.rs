use mlua::{Lua, Table};
use std::path::Path;

/// Lua Environment methods
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let env_table = lua.create_table()?;


    env_table.set(
        "get",
        lua.create_function(|_, (key, default): (String, Option<String>)| {
            if key.is_empty() {
                return Ok(default);
            }

            Ok(std::env::var_os(&key).map_or(default, |value| Some(value.to_string_lossy().into_owned())))
        })?,
    )?;

    env_table.set(
        "set",
        lua.create_function(|lua, (key, value): (String, String)| {
            super::require(lua, |p| p.allow_env_mutation, "environment mutation is disabled")?;
            if !is_valid_key(&key) || value.contains('\0') {
                return Ok(false);
            }

            // SAFETY: Environment changes are marked unsafe in std. Inputs are validated to avoid
            // interior NUL or unsupported keys before mutating the process environment.
            unsafe { std::env::set_var(key, value) };
            Ok(true)
        })?,
    )?;

    env_table.set(
        "unset",
        lua.create_function(|lua, key: String| {
            super::require(lua, |p| p.allow_env_mutation, "environment mutation is disabled")?;
            if !is_valid_key(&key) {
                return Ok(false);
            }

            // SAFETY: Environment mutation is unsafe in std; the key is validated to avoid UB.
            unsafe { std::env::remove_var(key) };
            Ok(true)
        })?,
    )?;

    env_table.set(
        "list",
        lua.create_function(|lua_ctx, ()| {
            let table = lua_ctx.create_table()?;

            for (key, value) in std::env::vars_os() {
                table.set(key.to_string_lossy(), value.to_string_lossy())?;
            }

            Ok(table)
        })?,
    )?;

    env_table.set(
        "is_exists",
        lua.create_function(|_, key: String| Ok(!key.is_empty() && std::env::var_os(key).is_some()))?,
    )?;

    env_table.set(
        "hostname",
        lua.create_function(|_, ()| {
            let name = hostname::get().map_or_else(|_| String::new(), |h| h.to_string_lossy().into_owned());
            Ok(name)
        })?,
    )?;

    env_table.set("which", lua.create_function(|_, name: String| Ok(which(&name)))?)?;
    env_table.set("is_in_path", lua.create_function(|_, name: String| Ok(which(&name).is_some()))?)?;

    Ok(env_table)
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('=') && !key.contains('\0') && !key.starts_with('=')
}

fn which(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }

    let path_exts = pathext();

    if contains_separator(name) {
        let path = Path::new(name);

        if path.is_absolute() {
            return probe_explicit(path, &path_exts);
        }

        if let Ok(current_dir) = std::env::current_dir() {
            let absolute = current_dir.join(path);
            return probe_explicit(&absolute, &path_exts);
        }

        return None;
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.as_os_str().is_empty() {
                continue;
            }

            if let Some(found) = probe_path(&dir, name, &path_exts) {
                return Some(found);
            }
        }
    }

    None
}

fn probe_explicit(path: &Path, exts: &[String]) -> Option<String> {
    // If the user gave an explicit path (absolute or relative), on Windows we still try PATHEXT
    // when no extension is provided.
    if candidate_is_executable(path, exts) {
        return Some(path.to_string_lossy().into_owned());
    }

    #[cfg(target_os = "windows")]
    {
        if path.extension().is_none() {
            for ext in exts {
                let mut with_ext = path.to_path_buf();
                // For "C:\bin\git" => "C:\bin\git.exe" etc.
                // ext includes the dot (".EXE")
                with_ext.set_extension(ext.trim_start_matches('.'));
                if candidate_is_executable(&with_ext, exts) {
                    return Some(with_ext.to_string_lossy().into_owned());
                }
            }
        }
    }

    None
}

fn probe_path(dir: &Path, name: &str, exts: &[String]) -> Option<String> {
    let base = dir.join(name);

    // If the name already has an extension, prefer checking the direct candidate first.
    if candidate_is_executable(&base, exts) {
        return Some(base.to_string_lossy().into_owned());
    }

    // Otherwise, try PATHEXT (Windows) or no-op list (Unix: exts is empty).
    for ext in exts {
        // ext already includes the dot: ".EXE"
        let with_ext = dir.join(format!("{name}{ext}"));
        if candidate_is_executable(&with_ext, exts) {
            return Some(with_ext.to_string_lossy().into_owned());
        }
    }

    None
}

#[allow(clippy::missing_const_for_fn)]
fn pathext() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("PATHEXT")
            .map(|exts| {
                exts.to_string_lossy()
                    .split(';')
                    .filter(|ext| !ext.is_empty())
                    .map(|ext| ext.trim_start_matches('.').to_ascii_uppercase())
                    .map(|ext| format!(".{ext}"))
                    .collect::<Vec<_>>()
            })
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                    ".VBS".to_string(),
                    ".VBE".to_string(),
                    ".JS".to_string(),
                    ".JSE".to_string(),
                    ".WSF".to_string(),
                    ".WSH".to_string(),
                    ".MSC".to_string(),
                ]
            })
    }

    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

#[allow(unused_variables)]
fn candidate_is_executable(path: &Path, exts: &[String]) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        if exts.is_empty() {
            return true;
        }

        let Some(ext) = path.extension() else { return false };
        let ext = format!(".{}", ext.to_string_lossy().to_ascii_uppercase());
        exts.iter().any(|e| e.eq_ignore_ascii_case(&ext))
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
}

fn contains_separator(name: &str) -> bool {
    name.contains(std::path::MAIN_SEPARATOR) || name.contains('/') || name.contains('\\')
}
