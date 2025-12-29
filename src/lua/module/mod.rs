#![allow(clippy::unnecessary_wraps, clippy::too_many_lines)]

use hex::ToHex;
use mlua::{Lua, MultiValue, Table, Value};
use rand::{RngCore, SeedableRng, rngs::SmallRng};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const STORE_SUBDIR: &str = ".store";
const TMP_SUBDIR: &str = ".tmp";

/// Preferred location for downloaded modules:
/// `{ward_data_dir}/externals/<name>`
fn externals_dir() -> PathBuf {
    crate::common::paths::data_dir().join("externals")
}

fn externals_store_dir() -> PathBuf {
    externals_dir().join(STORE_SUBDIR)
}

fn externals_tmp_dir() -> PathBuf {
    externals_store_dir().join(TMP_SUBDIR)
}

fn path_to_string(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn strip_query_and_fragment(s: &str) -> &str {
    s.split(['?', '#']).next().unwrap_or(s)
}

fn normalize_url(url: &str) -> String {
    strip_query_and_fragment(url.trim()).trim_end_matches('/').to_string()
}

fn derive_name_from_url(url: &str) -> String {
    let mut s = url.trim();
    s = strip_query_and_fragment(s);

    // Trim trailing slash.
    s = s.trim_end_matches('/');

    // Support scp-like git URLs: git@github.com:user/repo.git
    let s = s.rsplit(':').next().unwrap_or(s);
    let last = s.rsplit('/').next().unwrap_or(s);

    let last = last.strip_suffix(".git").unwrap_or(last);
    let last = last.strip_suffix(".lua").unwrap_or(last);
    let last = last.strip_suffix(".zip").unwrap_or(last);
    let last = last.strip_suffix(".tar.gz").unwrap_or(last);
    let last = last.strip_suffix(".tgz").unwrap_or(last);

    if last.is_empty() {
        "module".to_string()
    } else {
        last.to_string()
    }
}

fn canonicalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '.' || ch == ' ' {
            out.push('_');
        } else {
            // drop other characters
        }
    }

    // Collapse consecutive underscores.
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out = out.trim_matches('_').to_string();

    if out.is_empty() {
        return "module".to_string();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn is_safe_module_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Default)]
struct GitOpts {
    name: Option<String>,
    rev: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    force: bool,

    // pass-through (compatible with ward.net.fetch.git)
    depth: Option<u32>,
    recursive: Option<bool>,
    timeout: Option<Duration>,
    max_bytes: Option<u64>,
    filter_blobs: Option<bool>,
}

impl GitOpts {
    fn from_value(v: Value) -> mlua::Result<Self> {
        let mut o = Self::default();
        let Value::Table(t) = v else {
            return Ok(o);
        };

        o.name = t.get::<Option<String>>("name")?;
        o.rev = t.get::<Option<String>>("rev")?;
        o.branch = t.get::<Option<String>>("branch")?;
        o.tag = t.get::<Option<String>>("tag")?;
        o.force = t.get::<Option<bool>>("force")?.unwrap_or(false);

        o.depth = t.get::<Option<u32>>("depth")?;
        o.recursive = t.get::<Option<bool>>("recursive")?;
        o.filter_blobs = t.get::<Option<bool>>("filter_blobs")?;

        // timeout: seconds (float)
        if let Some(timeout) = t.get::<Option<f64>>("timeout")?
            && timeout.is_finite()
            && timeout.is_sign_positive()
        {
            o.timeout = Some(Duration::from_secs_f64(timeout));
        }

        // max_bytes: integer/number; <=0 disables
        if let Some(v) = t.get::<Option<Value>>("max_bytes")? {
            o.max_bytes = match v {
                #[allow(clippy::cast_sign_loss)]
                Value::Integer(i) if i > 0 => Some(i as u64),
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Value::Number(n) if n.is_finite() && n > 0.0 => Some(n as u64),
                _ => None,
            };
        }

        Ok(o)
    }
}

#[derive(Default)]
struct UrlOpts {
    name: Option<String>,
    insecure: bool,
    retries: u32,
    retry_delay_ms: u64,
    follow_redirects: bool,
    force: bool,

    // pass-through (compatible with ward.net.fetch.url)
    timeout: Option<Duration>,
    max_bytes: Option<u64>,
    headers: Vec<(String, String)>,
    method: Option<String>,
}

impl UrlOpts {
    fn from_value(v: Value) -> mlua::Result<Self> {
        let mut o = Self {
            retries: 5,
            retry_delay_ms: 2000,
            follow_redirects: true,
            ..Self::default()
        };
        let Value::Table(t) = v else {
            return Ok(o);
        };

        o.name = t.get::<Option<String>>("name")?;
        o.insecure = t.get::<Option<bool>>("insecure")?.unwrap_or(false);
        o.follow_redirects = t.get::<Option<bool>>("follow_redirects")?.unwrap_or(true);
        o.force = t.get::<Option<bool>>("force")?.unwrap_or(false);

        o.retries = t.get::<Option<u32>>("retries")?.unwrap_or(5).max(1);
        o.retry_delay_ms = t.get::<Option<u64>>("retry_delay")?.unwrap_or(2000);

        if let Some(timeout) = t.get::<Option<f64>>("timeout")?
            && timeout.is_finite()
            && timeout.is_sign_positive()
        {
            o.timeout = Some(Duration::from_secs_f64(timeout));
        }

        // max_bytes: integer/number; <=0 disables
        if let Some(v) = t.get::<Option<Value>>("max_bytes")? {
            o.max_bytes = match v {
                #[allow(clippy::cast_sign_loss)]
                Value::Integer(i) if i > 0 => Some(i as u64),
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Value::Number(n) if n.is_finite() && n > 0.0 => Some(n as u64),
                _ => None,
            };
        }

        o.method = t.get::<Option<String>>("method")?;

        if let Ok(headers_table) = t.get::<Table>("headers") {
            for entry in headers_table.pairs::<String, String>() {
                let (k, v) = entry?;
                o.headers.push((k, v));
            }
        }

        Ok(o)
    }
}

fn module_result(lua: &Lua, name: &str, id: &str, path: &Path, ok: bool) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("ok", ok)?;
    t.set("name", name.to_string())?;
    t.set("require", format!("externals.{name}"))?;
    t.set("path", path_to_string(path))?;
    t.set("store_path", path_to_string(path))?;
    t.set("id", id.to_string())?;
    Ok(t)
}

fn selector_from_git_opts(opts: &GitOpts) -> mlua::Result<String> {
    let mut selector: Option<String> = None;
    if let Some(rev) = &opts.rev {
        selector = Some(format!("rev:{rev}"));
    }
    if let Some(branch) = &opts.branch {
        if selector.is_some() {
            return Err(mlua::Error::external("only one of opts.rev/opts.branch/opts.tag may be set"));
        }
        selector = Some(format!("branch:{branch}"));
    }
    if let Some(tag) = &opts.tag {
        if selector.is_some() {
            return Err(mlua::Error::external("only one of opts.rev/opts.branch/opts.tag may be set"));
        }
        selector = Some(format!("tag:{tag}"));
    }

    Ok(selector.unwrap_or_else(|| "head".to_string()))
}

fn store_id(url: &str, selector: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_url(url).as_bytes());
    hasher.update(b"\n");
    hasher.update(selector.as_bytes());
    hasher.finalize().encode_hex::<String>()
}

fn store_path_for_id(id: &str) -> PathBuf {
    externals_store_dir().join(id)
}

fn random_tmp_dir() -> PathBuf {
    let mut rng = SmallRng::from_os_rng();
    let mut buf = [0u8; 8];
    rng.fill_bytes(&mut buf);
    externals_tmp_dir().join(buf.encode_hex::<String>())
}

fn externals_binding_map(lua: &Lua) -> mlua::Result<Table> {
    let globals = lua.globals();
    if let Ok(t) = globals.get::<Table>("__ward_externals_map") {
        return Ok(t);
    }

    let t = lua.create_table()?;
    globals.set("__ward_externals_map", t.clone())?;
    Ok(t)
}

fn bind_external(lua: &Lua, name: &str, id: &str, force: bool) -> mlua::Result<()> {
    let map = externals_binding_map(lua)?;

    if let Some(existing) = map.get::<Option<String>>(name)? {
        if existing == id {
            return Ok(());
        }
        if !force {
            return Err(mlua::Error::external(format!(
                "external '{name}' already bound to {existing}; set opts.force=true to rebind"
            )));
        }
        let package: Table = lua.globals().get("package")?;
        let loaded: Table = package.get("loaded")?;
        loaded.set(format!("externals.{name}"), Value::Nil)?;
    }

    map.set(name, id)?;
    Ok(())
}

/// Installs a dedicated `package.searchers` entry for `externals.*`.
///
/// This makes `require("externals.<name>")` load from the content-addressed
/// externals store with per-run bindings.
/// # Errors [`mlua::Error`]
pub fn install_externals_searcher(lua: &Lua) -> mlua::Result<()> {
    // Idempotency marker.
    if lua
        .globals()
        .get::<Option<bool>>("__ward_externals_searcher_installed")?
        .unwrap_or(false)
    {
        return Ok(());
    }

    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package.get("searchers")?;

    let _ = externals_binding_map(lua)?;
    let store_dir = externals_store_dir();
    let store_dir_s = path_to_string(&store_dir);

    // Searcher signature: function(modname) -> loader, filepath OR error_message
    let searcher = lua.create_function(move |lua, modname: String| -> mlua::Result<MultiValue> {
        if !modname.starts_with("externals.") {
            // Not our prefix: return nil to let other searchers continue.
            return Ok(MultiValue::new());
        }

        let parts: Vec<&str> = modname.split('.').collect();
        if parts.len() < 2 {
            return Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string("\n\tinvalid externals module name")?,
            )]));
        }
        let root_name = parts.get(1).copied().unwrap_or_default();
        if !is_safe_module_segment(root_name) {
            return Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string("\n\tinvalid externals module name")?,
            )]));
        }

        // Validate submodule segments to prevent path traversal.
        for seg in parts.iter().skip(2) {
            if !is_safe_module_segment(seg) {
                return Ok(MultiValue::from_vec(vec![Value::String(
                    lua.create_string("\n\tinvalid externals module name")?,
                )]));
            }
        }

        let bindings: Table = lua
            .globals()
            .get("__ward_externals_map")
            .map_err(|e| mlua::Error::external(format!("externals bindings unavailable: {e}")))?;
        let Some(store_id) = bindings.get::<Option<String>>(root_name)? else {
            return Ok(MultiValue::from_vec(vec![Value::String(lua.create_string(format!(
                "\n\tno externals binding for '{root_name}' (call ward.module.git/url first)"
            ))?)]));
        };

        // Build candidates.
        let root = PathBuf::from(&store_dir_s).join(store_id);
        if !root.exists() {
            return Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string(format!("\n\tno externals module '{modname}' in {store_dir_s}"))?,
            )]));
        }
        let rel = if parts.len() > 2 {
            parts[2..].join("/")
        } else {
            String::new()
        };

        let mut candidates: Vec<PathBuf> = Vec::new();

        if rel.is_empty() {
            candidates.push(root.join("init.lua"));
            candidates.push(root.join(format!("{root_name}.lua")));
            candidates.push(root.join("lua").join("init.lua"));
            candidates.push(root.join("lua").join(format!("{root_name}.lua")));
            candidates.push(root.join("lua").join(root_name).join("init.lua"));
        } else {
            candidates.push(root.join(format!("{rel}.lua")));
            candidates.push(root.join(&rel).join("init.lua"));
            candidates.push(root.join("lua").join(format!("{rel}.lua")));
            candidates.push(root.join("lua").join(&rel).join("init.lua"));
            candidates.push(root.join("lua").join(root_name).join(format!("{rel}.lua")));
            candidates.push(root.join("lua").join(root_name).join(&rel).join("init.lua"));
        }

        for cand in candidates {
            // Reject symlinks to avoid escaping the externals root.
            let Ok(meta) = std::fs::symlink_metadata(&cand) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                continue;
            }

            {
                let content = std::fs::read_to_string(&cand)
                    .map_err(|e| mlua::Error::external(format!("failed to read {}: {e}", path_to_string(&cand))))?;
                let name = path_to_string(&cand);
                let loader = lua.load(&content).set_name(&name).into_function()?;
                let p = lua.create_string(&name)?;
                return Ok(MultiValue::from_vec(vec![Value::Function(loader), Value::String(p)]));
            }
        }

        Ok(MultiValue::from_vec(vec![Value::String(lua.create_string(format!(
            "\n\tno externals module '{modname}' in {store_dir_s}"
        ))?)]))
    })?;

    // Insert right after the preload searcher (position 2 in Lua 5.4).
    let len = searchers.raw_len();
    // Shift existing entries by 1 starting from the end.
    for i in (2..=len).rev() {
        let v: Value = searchers.get(i)?;
        searchers.set(i + 1, v)?;
    }
    searchers.set(2, searcher)?;

    lua.globals().set("__ward_externals_searcher_installed", true)?;

    Ok(())
}

/// Initializes the `module` module.
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    m.set("dir", lua.create_function(|_, ()| Ok(path_to_string(&externals_dir())))?)?;

    // module.git(url, opts?) -> { ok, name, require, path, store_path, id }
    m.set(
        "git",
        lua.create_async_function(|lua, (url, opts): (String, Value)| async move {
            let opts = GitOpts::from_value(opts)?;
            let raw_name = opts.name.clone().unwrap_or_else(|| derive_name_from_url(&url));
            let name = canonicalize_name(&raw_name);
            let selector = selector_from_git_opts(&opts)?;
            let id = store_id(&url, &selector);
            let target = store_path_for_id(&id);

            tokio::fs::create_dir_all(externals_store_dir())
                .await
                .map_err(mlua::Error::external)?;
            tokio::fs::create_dir_all(externals_tmp_dir())
                .await
                .map_err(mlua::Error::external)?;

            if !target.exists() {
                let tmp_dir = random_tmp_dir();
                let tmp_parent = tmp_dir
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(externals_tmp_dir);
                tokio::fs::create_dir_all(tmp_parent)
                    .await
                    .map_err(mlua::Error::external)?;

                let overlay = crate::lua::env::overlay_snapshot(&lua)?;

                // Reuse ward.net.fetch.git engine.
                let t = lua.create_table()?;
                t.set("into", path_to_string(&tmp_dir))?;
                if let Some(v) = opts.rev {
                    t.set("rev", v)?;
                }
                if let Some(v) = opts.branch {
                    t.set("branch", v)?;
                }
                if let Some(v) = opts.tag {
                    t.set("tag", v)?;
                }
                if let Some(v) = opts.depth {
                    t.set("depth", v)?;
                }
                if let Some(v) = opts.recursive {
                    t.set("recursive", v)?;
                }
                if let Some(v) = opts.max_bytes {
                    t.set("max_bytes", v)?;
                }
                if let Some(v) = opts.filter_blobs {
                    t.set("filter_blobs", v)?;
                }
                if let Some(v) = opts.timeout {
                    t.set("timeout", v.as_secs_f64())?;
                }

                let resp = crate::lua::net::fetch::fetch_git_async(&url, Value::Table(t), overlay).await?;
                if !resp.ok {
                    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                    return module_result(&lua, &name, &id, &target, false);
                }

                let rename_result = tokio::fs::rename(&tmp_dir, &target).await;
                if let Err(e) = rename_result {
                    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                    if target.exists() {
                        // Another process installed it first; treat as success.
                    } else {
                        return Err(mlua::Error::external(format!("failed to finalize git checkout: {e}")));
                    }
                }
            }

            bind_external(&lua, &name, &id, opts.force)?;
            module_result(&lua, &name, &id, &target, true)
        })?,
    )?;

    // module.url(url, opts?) -> { ok, name, require, path, store_path, id }
    m.set(
        "url",
        lua.create_async_function(|lua, (url, opts): (String, Value)| async move {
            let opts = UrlOpts::from_value(opts)?;
            let raw_name = opts.name.clone().unwrap_or_else(|| derive_name_from_url(&url));
            let name = canonicalize_name(&raw_name);
            let id = store_id(&url, "url");
            let target_dir = store_path_for_id(&id);

            tokio::fs::create_dir_all(externals_store_dir())
                .await
                .map_err(mlua::Error::external)?;
            tokio::fs::create_dir_all(externals_tmp_dir())
                .await
                .map_err(mlua::Error::external)?;

            if !target_dir.exists() {
                let tmp_dir = random_tmp_dir();
                tokio::fs::create_dir_all(&tmp_dir)
                    .await
                    .map_err(mlua::Error::external)?;

                let tmp_file = tmp_dir.join("init.lua");

                // Prepare options for ward.net.fetch.url.
                let t = lua.create_table()?;
                t.set("into", path_to_string(&tmp_file))?;
                t.set("follow_redirects", opts.follow_redirects)?;
                t.set("insecure", opts.insecure)?;
                if let Some(v) = opts.max_bytes {
                    t.set("max_bytes", v)?;
                }
                if let Some(v) = opts.timeout {
                    t.set("timeout", v.as_secs_f64())?;
                }
                if let Some(v) = opts.method {
                    t.set("method", v)?;
                }
                if !opts.headers.is_empty() {
                    let map = lua.create_table()?;
                    for (k, v) in &opts.headers {
                        map.set(k.clone(), v.clone())?;
                    }
                    t.set("headers", map)?;
                }

                let mut attempt: u32 = 1;
                loop {
                    let resp = crate::lua::net::fetch::fetch_url_async(&url, Value::Table(t.clone())).await?;
                    if resp.ok {
                        break;
                    }

                    // Best-effort cleanup of a partial/failed download.
                    let _ = tokio::fs::remove_file(&tmp_file).await;

                    if attempt >= opts.retries {
                        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                        return module_result(&lua, &name, &id, &target_dir, false);
                    }
                    attempt += 1;
                    if opts.retry_delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(opts.retry_delay_ms)).await;
                    }
                }

                let rename_result = tokio::fs::rename(&tmp_dir, &target_dir).await;
                if let Err(e) = rename_result {
                    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                    if target_dir.exists() {
                        // Another process completed first.
                    } else {
                        return Err(mlua::Error::external(format!("failed to finalize url download: {e}")));
                    }
                }
            }

            bind_external(&lua, &name, &id, opts.force)?;
            module_result(&lua, &name, &id, &target_dir, true)
        })?,
    )?;

    Ok(m)
}
