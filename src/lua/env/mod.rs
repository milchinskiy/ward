use mlua::{Lua, Table};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// Ward-local environment overlay.
/// - `ward.env.set/unset` affect only this overlay
/// - `ward.env.get/list/is_exists/which/is_in_path` resolve the overlay first
/// - child process spawning can apply the overlay to the `Command` environment
#[derive(Clone, Default)]
pub struct EnvOverlay {
    inner: Arc<RwLock<HashMap<String, Option<String>>>>,
}

impl EnvOverlay {
    pub fn set(&self, key: String, value: String) {
        let mut map = self.inner.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.insert(key, Some(value));
    }

    pub fn unset(&self, key: String) {
        let mut map = self.inner.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.insert(key, None);
    }

    pub fn clear(&self) {
        let mut map = self.inner.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.clear();
    }

    /// Returns:
    /// - `Some(Some(v))` if overridden to `v`
    /// - `Some(None)` if explicitly unset
    /// - `None` if not present in overlay
    pub fn lookup(&self, key: &str) -> Option<Option<String>> {
        let map = self.inner.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(key).cloned()
    }

    pub fn snapshot(&self) -> HashMap<String, Option<String>> {
        let map = self.inner.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        map.clone()
    }
}

fn ensure_overlay(lua: &Lua) -> EnvOverlay {
    lua.app_data_ref::<EnvOverlay>().map_or_else(
        || {
            let o = EnvOverlay::default();
            lua.set_app_data(o.clone());
            o
        },
        |o| o.clone(),
    )
}

fn warn_export_once() {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        rustlog::warn!("ward.env.export mutates the process environment; prefer ward.env.set for isolated changes");
    }
}

/// Returns a snapshot of the current overlay.
///
/// This is used by subprocess spawners to apply the overlay to child environments.
/// # Errors [`mlua::Error`]
pub fn overlay_snapshot(lua: &Lua) -> mlua::Result<HashMap<String, Option<String>>> {
    Ok(ensure_overlay(lua).snapshot())
}

fn effective_var_os(overlay: &EnvOverlay, key: &str) -> Option<OsString> {
    match overlay.lookup(key) {
        Some(Some(v)) => Some(OsString::from(v)),
        Some(None) => None,
        None => std::env::var_os(key),
    }
}

fn effective_var_string(overlay: &EnvOverlay, key: &str) -> Option<String> {
    effective_var_os(overlay, key).map(|v| v.to_string_lossy().into_owned())
}

fn effective_env_map(overlay: &EnvOverlay) -> HashMap<String, String> {
    // Start with the real environment.
    let mut out: HashMap<String, String> = HashMap::new();
    for (k, v) in std::env::vars_os() {
        out.insert(k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned());
    }

    // Apply overlay modifications.
    for (k, vv) in overlay.snapshot() {
        match vv {
            Some(v) => {
                out.insert(k, v);
            }
            None => {
                out.remove(&k);
            }
        }
    }

    out
}

/// Lua Environment methods
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let overlay = ensure_overlay(lua);
    let env_table = lua.create_table()?;

    env_table.set(
        "get",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, (key, default): (String, Option<String>)| {
                if key.is_empty() {
                    return Ok(default);
                }
                Ok(effective_var_string(&overlay, &key).or(default))
            }
        })?,
    )?;

    env_table.set(
        "set",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, (key, value): (String, String)| {
                if !is_valid_key(&key) || value.contains('\0') {
                    return Ok(false);
                }
                overlay.set(key, value);
                Ok(true)
            }
        })?,
    )?;

    env_table.set(
        "unset",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, key: String| {
                if !is_valid_key(&key) {
                    return Ok(false);
                }
                overlay.unset(key);
                Ok(true)
            }
        })?,
    )?;

    env_table.set(
        "clear",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, ()| {
                overlay.clear();
                Ok(())
            }
        })?,
    )?;

    env_table.set(
        "export",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, (key, value): (String, Option<String>)| {
                if !is_valid_key(&key) {
                    return Ok(false);
                }
                if value.as_ref().is_some_and(|v| v.contains('\0')) {
                    return Ok(false);
                }
                warn_export_once();
                if let Some(v) = value {
                    // SAFETY: std::env::set_var is marked unsafe on this target; the caller
                    // explicitly requested process-wide mutation via `export`.
                    unsafe { std::env::set_var(&key, &v) };
                    overlay.set(key, v);
                } else {
                    // SAFETY: same rationale as set_var above; removing mirrors shell `unset`.
                    unsafe { std::env::remove_var(&key) };
                    overlay.unset(key);
                }
                Ok(true)
            }
        })?,
    )?;

    env_table.set(
        "list",
        lua.create_function({
            let overlay = overlay.clone();
            move |lua_ctx, ()| {
                let table = lua_ctx.create_table()?;
                for (k, v) in effective_env_map(&overlay) {
                    table.set(k, v)?;
                }
                Ok(table)
            }
        })?,
    )?;

    env_table.set(
        "is_exists",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, key: String| {
                if key.is_empty() {
                    return Ok(false);
                }
                match overlay.lookup(&key) {
                    Some(Some(_)) => Ok(true),
                    Some(None) => Ok(false),
                    None => Ok(std::env::var_os(key).is_some()),
                }
            }
        })?,
    )?;

    env_table.set(
        "hostname",
        lua.create_function(|_, ()| {
            let name = hostname::get().map_or_else(|_| String::new(), |h| h.to_string_lossy().into_owned());
            Ok(name)
        })?,
    )?;

    env_table.set(
        "which",
        lua.create_function({
            let overlay = overlay.clone();
            move |_, name: String| {
                let path = effective_var_os(&overlay, "PATH");
                let pathext = effective_var_os(&overlay, "PATHEXT");
                Ok(which_with_env(&name, path.as_deref(), pathext.as_deref()))
            }
        })?,
    )?;

    env_table.set(
        "is_in_path",
        lua.create_function({
            move |_, name: String| {
                let path = effective_var_os(&overlay, "PATH");
                let pathext = effective_var_os(&overlay, "PATHEXT");
                Ok(which_with_env(&name, path.as_deref(), pathext.as_deref()).is_some())
            }
        })?,
    )?;

    Ok(env_table)
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && !key.contains('=') && !key.contains('\0') && !key.starts_with('=')
}

fn which_with_env(name: &str, path: Option<&OsStr>, pathext_var: Option<&OsStr>) -> Option<String> {
    if name.is_empty() {
        return None;
    }

    let path_exts = pathext(pathext_var);

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

    if let Some(paths) = path {
        for dir in std::env::split_paths(paths) {
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
                // For "C:\\bin\\git" => "C:\\bin\\git.exe" etc.
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
fn pathext(var: Option<&OsStr>) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        var.map(|exts| {
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
        let _ = var;
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
