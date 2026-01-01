#![allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]

use std::path::{Path, PathBuf};

use mlua::{Lua, LuaSerdeExt, Table, Value};

/// Initializes the `ward.template.minijinja` module.
///
/// Functions:
/// - `render(template, context, opts?) -> string`
/// - `render_async(template, context, opts?) -> string`
/// - `render_file(path, context, opts?) -> string`
/// - `render_file_async(path, context, opts?) -> string`
///
/// Options (`opts`):
/// - `undefined` (`"strict"|"lenient"|"chainable"`), default: `"strict"`
/// - `trim_blocks` (bool), default: `false`
/// - `lstrip_blocks` (bool), default: `false`
/// - `keep_trailing_newline` (bool), default: `false`
/// - `auto_escape` (bool), default: `false`
/// - `loader` (table), default: `nil`
///   - `paths` (array of strings) – search paths for `{% include %}` / `{% import %}`.
///
/// Notes:
/// - Context is converted via Serde; any Lua table composed of JSON-like scalars/arrays/maps is supported.
/// - Rendering can be CPU-heavy; async variants execute on a blocking thread.
///
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "render",
        lua.create_function(|lua, (template, context, opts): (String, Value, Option<Table>)| {
            render_str(lua, &template, context, opts)
        })?,
    )?;

    t.set(
        "render_async",
        lua.create_async_function(|lua, (template, context, opts): (String, Value, Option<Table>)| async move {
            let ctx = lua.from_value::<serde_json::Value>(context)?;
            let opts = parse_options(opts)?;
            tokio::task::spawn_blocking(move || render_str_send(&template, ctx, opts))
                .await
                .map_err(mlua::Error::external)?
                .map_err(mlua::Error::external)
        })?,
    )?;

    t.set(
        "render_file",
        lua.create_function(|lua, (path, context, opts): (String, Value, Option<Table>)| {
            render_file(lua, PathBuf::from(path), context, opts)
        })?,
    )?;

    t.set(
        "render_file_async",
        lua.create_async_function(|lua, (path, context, opts): (String, Value, Option<Table>)| async move {
            let path = PathBuf::from(path);
            let ctx = lua.from_value::<serde_json::Value>(context)?;
            let opts = parse_options(opts)?;

            tokio::task::spawn_blocking(move || render_file_send(&path, ctx, opts))
                .await
                .map_err(mlua::Error::external)?
                .map_err(mlua::Error::external)
        })?,
    )?;

    Ok(t)
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
struct Options {
    undefined: minijinja::UndefinedBehavior,
    trim_blocks: bool,
    lstrip_blocks: bool,
    keep_trailing_newline: bool,
    auto_escape: bool,
    loader_paths: Vec<PathBuf>,
}

fn parse_options(opts: Option<Table>) -> mlua::Result<Options> {
    let Some(t) = opts else {
        return Ok(Options {
            undefined: minijinja::UndefinedBehavior::Strict,
            trim_blocks: false,
            lstrip_blocks: false,
            keep_trailing_newline: false,
            auto_escape: false,
            loader_paths: Vec::new(),
        });
    };

    let undefined = match t.get::<Option<String>>("undefined")?.as_deref() {
        None | Some("strict") => minijinja::UndefinedBehavior::Strict,
        Some("lenient") => minijinja::UndefinedBehavior::Lenient,
        Some("chainable") => minijinja::UndefinedBehavior::Chainable,
        Some(other) => {
            return Err(mlua::Error::external(format!(
                "unknown undefined behavior: {other} (expected: strict|lenient|chainable)"
            )));
        }
    };

    let trim_blocks = t.get::<Option<bool>>("trim_blocks")?.unwrap_or(false);
    let lstrip_blocks = t.get::<Option<bool>>("lstrip_blocks")?.unwrap_or(false);
    let keep_trailing_newline = t.get::<Option<bool>>("keep_trailing_newline")?.unwrap_or(false);
    let auto_escape = t.get::<Option<bool>>("auto_escape")?.unwrap_or(false);

    // Loader configuration is optional. It is only used for file-based rendering (and for includes/imports).
    let loader_paths = if let Ok(loader) = t.get::<Table>("loader") {
        parse_loader_paths(loader)?
    } else {
        Vec::new()
    };

    Ok(Options {
        undefined,
        trim_blocks,
        lstrip_blocks,
        keep_trailing_newline,
        auto_escape,
        loader_paths,
    })
}

fn parse_loader_paths(loader: Table) -> mlua::Result<Vec<PathBuf>> {
    let paths_value = loader.get::<Option<Table>>("paths")?;
    let Some(paths_table) = paths_value else {
        return Ok(Vec::new());
    };

    let mut paths = Vec::new();
    for p in paths_table.sequence_values::<String>() {
        paths.push(PathBuf::from(p?));
    }
    Ok(paths)
}

fn build_env(opts: &Options, main_dir: Option<&Path>) -> Result<minijinja::Environment<'static>, String> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(opts.undefined);
    env.set_trim_blocks(opts.trim_blocks);
    env.set_lstrip_blocks(opts.lstrip_blocks);
    env.set_keep_trailing_newline(opts.keep_trailing_newline);

    if opts.auto_escape {
        // Minijinja supports auto-escaping; for now, use the built-in HTML auto-escape.
        env.set_auto_escape_callback(|name| {
            let _ = name;
            minijinja::AutoEscape::Html
        });
    }

    // If loader paths are configured (or main_dir is provided), install a simple filesystem loader.
    // This enables `{% include %}` and `{% import %}`.
    if main_dir.is_some() || !opts.loader_paths.is_empty() {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(d) = main_dir {
            roots.push(d.to_path_buf());
        }
        roots.extend(opts.loader_paths.iter().cloned());

        env.set_loader(move |name| load_template_from_roots(name, &roots).map(Some));
    }

    Ok(env)
}

fn load_template_from_roots(name: &str, roots: &[PathBuf]) -> Result<String, minijinja::Error> {
    for root in roots {
        let candidate = root.join(name);
        if let Ok(src) = std::fs::read_to_string(&candidate) {
            return Ok(src);
        }
    }

    Err(minijinja::Error::new(
        minijinja::ErrorKind::TemplateNotFound,
        format!("template not found: {name}"),
    ))
}

fn render_str(lua: &Lua, template: &str, context: Value, opts: Option<Table>) -> mlua::Result<String> {
    let ctx = lua.from_value::<serde_json::Value>(context)?;
    let opts = parse_options(opts)?;
    render_str_send(template, ctx, opts).map_err(mlua::Error::external)
}

fn render_str_send(template: &str, ctx: serde_json::Value, opts: Options) -> Result<String, String> {
    let env = build_env(&opts, None)?;
    env.render_str(template, ctx).map_err(|e| e.to_string())
}

fn render_file(lua: &Lua, path: PathBuf, context: Value, opts: Option<Table>) -> mlua::Result<String> {
    let ctx = lua.from_value::<serde_json::Value>(context)?;
    let opts = parse_options(opts)?;
    render_file_send(&path, ctx, opts).map_err(mlua::Error::external)
}

fn render_file_send(path: &Path, ctx: serde_json::Value, opts: Options) -> Result<String, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}", path = path.display()))?;
    let main_dir = path.parent();
    let env = build_env(&opts, main_dir)?;
    env.render_str(&src, ctx).map_err(|e| e.to_string())
}
