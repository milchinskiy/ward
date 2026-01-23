#![allow(clippy::unnecessary_wraps, clippy::too_many_lines)]

use hex::ToHex;
use mlua::{Lua, Table, Value};
use rand::{RngCore, SeedableRng, rngs::SmallRng};
use sha2::{Digest, Sha256};
use std::future::Future;
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
    t.set("require", name.to_string())?;
    t.set("path", path_to_string(path))?;
    t.set("store_path", path_to_string(path))?;
    t.set("id", id.to_string())?;
    Ok(t)
}

fn selector_from_git_opts(opts: &GitOpts) -> mlua::Result<String> {
    let mut selector = opts.rev.as_ref().map(|rev| format!("rev:{rev}"));
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

fn normalize_lua_path(p: &str) -> String {
    // Lua package.path patterns typically use forward slashes even on Windows.
    p.replace('\\', "/")
}

fn externals_path_map(lua: &Lua) -> mlua::Result<Table> {
    let globals = lua.globals();
    if let Ok(t) = globals.get::<Table>("__ward_externals_path_map") {
        return Ok(t);
    }

    let t = lua.create_table()?;
    globals.set("__ward_externals_path_map", t.clone())?;
    Ok(t)
}

fn external_package_patterns(root: &str) -> Vec<String> {
    let r = normalize_lua_path(root).trim_end_matches('/').to_string();
    vec![
        format!("{r}/?.lua"),
        format!("{r}/?/init.lua"),
        format!("{r}/lua/?.lua"),
        format!("{r}/lua/?/init.lua"),
    ]
}

fn remove_from_package_path(package: &Table, patterns: &[String]) -> mlua::Result<()> {
    let path: String = package.get("path")?;
    let mut parts: Vec<String> = path
        .split(';')
        .filter(|p| !p.is_empty())
        .map(std::string::ToString::to_string)
        .collect();

    parts.retain(|p| !patterns.iter().any(|x| x == p));
    package.set("path", parts.join(";"))?;
    Ok(())
}

fn insert_into_package_path(package: &Table, patterns: &[String]) -> mlua::Result<()> {
    let path: String = package.get("path")?;
    let mut parts: Vec<String> = path
        .split(';')
        .filter(|p| !p.is_empty())
        .map(std::string::ToString::to_string)
        .collect();

    // Remove duplicates first (idempotent updates).
    parts.retain(|p| !patterns.iter().any(|x| x == p));

    // Insert after CWD patterns if present (Ward adds ./?.lua and ./?/init.lua early).
    let mut insert_at: usize = 0;
    for (i, p) in parts.iter().enumerate() {
        if p == "./?.lua" || p == "./?/init.lua" {
            insert_at = i + 1;
        }
    }

    for (idx, pat) in patterns.iter().enumerate() {
        parts.insert(insert_at + idx, pat.clone());
    }

    package.set("path", parts.join(";"))?;
    Ok(())
}

fn clear_loaded_for_external(lua: &Lua, name: &str) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let loaded: Table = package.get("loaded")?;

    let mut to_remove: Vec<String> = Vec::new();
    let prefix = format!("{name}.");

    for pair in loaded.pairs::<String, Value>() {
        let (k, _) = pair?;
        if k == name || k.starts_with(&prefix) {
            to_remove.push(k);
        }
    }

    for k in to_remove {
        loaded.set(k, Value::Nil)?;
    }
    Ok(())
}

fn bind_external(lua: &Lua, name: &str, id: &str, store_path: &Path, force: bool) -> mlua::Result<()> {
    let map = externals_binding_map(lua)?;
    let path_map = externals_path_map(lua)?;

    if let Some(existing) = map.get::<Option<String>>(name)?
        && existing != id
    {
        if !force {
            return Err(mlua::Error::external(format!(
                "external '{name}' already bound to {existing}; set opts.force=true to rebind"
            )));
        }
        clear_loaded_for_external(lua, name)?;
    }

    // Remove old store root patterns on rebind (or if a previous bind exists in this VM).
    if let Some(old_root) = path_map.get::<Option<String>>(name)? {
        let old_patterns = external_package_patterns(&old_root);
        let package: Table = lua.globals().get("package")?;
        remove_from_package_path(&package, &old_patterns)?;
    }

    let root_s = path_to_string(store_path);
    let package: Table = lua.globals().get("package")?;
    insert_into_package_path(&package, &external_package_patterns(&root_s))?;

    path_map.set(name, root_s)?;
    map.set(name, id)?;
    Ok(())
}

fn run_install_async<T, F>(fut: F) -> mlua::Result<T>
where
    T: Send + 'static,
    F: Future<Output = mlua::Result<T>> + Send + 'static,
{
    // Always run module install work on a dedicated thread with its own Tokio runtime.
    //
    // Rationale:
    // - `ward.module.git/url()` must not yield in Lua (must be safe under `require(...)`).
    // - Blocking inside an existing Tokio runtime is fragile: `block_in_place` panics on
    //   current-thread runtimes (used by some tests) and `Handle::block_on` can deadlock.
    //
    // Overhead is acceptable because module install is inherently heavyweight (network + disk).
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(mlua::Error::external)?;
        rt.block_on(fut)
    })
    .join()
    .map_err(|_| mlua::Error::external("module install thread panicked"))?
}

/// Initializes the `module` module.
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    m.set("dir", lua.create_function(|_, ()| Ok(path_to_string(&externals_dir())))?)?;

    // module.git(url, opts?) -> { ok, name, require, path, store_path, id }
    //
    // NOTE: This function is intentionally synchronous from Lua's perspective.
    // Many users call it during `require(...)` (module initialization), and yielding
    // inside Lua's built-in `require` triggers `attempt to yield across a C-call boundary`.
    // We therefore run the async install pipeline to completion via a blocking bridge.
    m.set(
        "git",
        lua.create_function(|lua, (url, opts_v): (String, Value)| {
            let opts = GitOpts::from_value(opts_v)?;
            let raw_name = opts.name.clone().unwrap_or_else(|| derive_name_from_url(&url));
            let name = canonicalize_name(&raw_name);
            let selector = selector_from_git_opts(&opts)?;
            let id = store_id(&url, &selector);
            let target = store_path_for_id(&id);

            std::fs::create_dir_all(externals_store_dir()).map_err(mlua::Error::external)?;
            std::fs::create_dir_all(externals_tmp_dir()).map_err(mlua::Error::external)?;

            let ok = if target.exists() {
                true
            } else {
                let tmp_dir = random_tmp_dir();
                let tmp_parent = tmp_dir
                    .parent()
                    .map_or_else(externals_tmp_dir, std::path::Path::to_path_buf);
                std::fs::create_dir_all(&tmp_parent).map_err(mlua::Error::external)?;

                let overlay = crate::lua::env::overlay_snapshot(lua)?;

                let fopts = crate::lua::net::fetch::GitOptions {
                    into: Some(tmp_dir.clone()),
                    rev: opts.rev.clone(),
                    branch: opts.branch.clone(),
                    tag: opts.tag.clone(),
                    depth: opts.depth,
                    recursive: opts.recursive.unwrap_or(false),
                    timeout: opts.timeout,
                    max_bytes: opts.max_bytes,
                    filter_blobs: opts.filter_blobs.unwrap_or(true),
                };

                let target2 = target.clone();

                run_install_async(async move {
                    let resp = crate::lua::net::fetch::fetch_git_with_options_async(&url, fopts, overlay).await?;
                    if !resp.ok {
                        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                        return Ok(false);
                    }

                    let rename_result = tokio::fs::rename(&tmp_dir, &target2).await;
                    if let Err(e) = rename_result {
                        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                        if !target2.exists() {
                            return Err(mlua::Error::external(format!("failed to finalize git checkout: {e}")));
                        }
                    }

                    Ok(true)
                })?
            };

            if ok {
                bind_external(lua, &name, &id, &target, opts.force)?;
            }
            module_result(lua, &name, &id, &target, ok)
        })?,
    )?;

    // module.url(url, opts?) -> { ok, name, require, path, store_path, id }
    //
    // NOTE: Synchronous (see module.git note above).
    m.set(
        "url",
        lua.create_function(|lua, (url, opts_v): (String, Value)| {
            let opts = UrlOpts::from_value(opts_v)?;
            let raw_name = opts.name.clone().unwrap_or_else(|| derive_name_from_url(&url));
            let name = canonicalize_name(&raw_name);
            let id = store_id(&url, "url");
            let target_dir = store_path_for_id(&id);

            std::fs::create_dir_all(externals_store_dir()).map_err(mlua::Error::external)?;
            std::fs::create_dir_all(externals_tmp_dir()).map_err(mlua::Error::external)?;

            let ok = if target_dir.exists() {
                true
            } else {
                let tmp_dir = random_tmp_dir();
                std::fs::create_dir_all(&tmp_dir).map_err(mlua::Error::external)?;
                let tmp_file = tmp_dir.join("init.lua");

                let fopts = crate::lua::net::fetch::UrlOptions {
                    into: Some(tmp_file.clone()),
                    follow_redirects: opts.follow_redirects,
                    insecure: opts.insecure,
                    timeout: opts.timeout,
                    max_bytes: opts.max_bytes,
                    method: opts.method.clone().unwrap_or_else(|| "GET".to_string()),
                    headers: opts.headers.clone(),
                };

                let target2 = target_dir.clone();
                let retries = opts.retries;
                let retry_delay_ms = opts.retry_delay_ms;

                run_install_async(async move {
                    let mut attempt: u32 = 1;
                    loop {
                        let resp = crate::lua::net::fetch::fetch_url_with_options_async(&url, fopts.clone()).await?;
                        if resp.ok {
                            break;
                        }

                        let _ = tokio::fs::remove_file(&tmp_file).await;
                        if attempt >= retries {
                            let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                            return Ok(false);
                        }
                        attempt += 1;
                        if retry_delay_ms > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(retry_delay_ms)).await;
                        }
                    }

                    let rename_result = tokio::fs::rename(&tmp_dir, &target2).await;
                    if let Err(e) = rename_result {
                        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
                        if !target2.exists() {
                            return Err(mlua::Error::external(format!("failed to finalize url download: {e}")));
                        }
                    }

                    Ok(true)
                })?
            };

            if ok {
                bind_external(lua, &name, &id, &target_dir, opts.force)?;
            }
            module_result(lua, &name, &id, &target_dir, ok)
        })?,
    )?;

    Ok(m)
}
