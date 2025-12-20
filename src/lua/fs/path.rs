#![allow(clippy::needless_pass_by_value, clippy::missing_const_for_fn)]

use std::path::{Component, Path, PathBuf};

use mlua::{AnyUserData, Lua, MetaMethod, Table, UserData, UserDataMethods, Value};

#[derive(Clone, Debug)]
pub(crate) struct PathObj {
    pub(crate) path: PathBuf,
}

impl PathObj {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn as_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl UserData for PathObj {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("is_abs", |_, this, ()| Ok(this.path.is_absolute()));
        methods.add_method("normalize", |_, this, ()| Ok(Self::new(normalize_path(&this.path))));
        methods.add_method("parts", |lua, this, ()| {
            let t = lua.create_table()?;
            for (idx, comp) in this.path.components().enumerate() {
                t.set(idx + 1, component_to_string(comp))?;
            }
            Ok(t)
        });

        // Returns (dirname, basename)
        methods.add_method("split", |_, this, ()| {
            let dir = this
                .path
                .parent()
                .map_or_else(|| String::from("."), |p| p.to_string_lossy().into_owned());
            let base = this
                .path
                .file_name()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            Ok((dir, base))
        });

        methods.add_method("join", |_, this, seg: Value| {
            let seg = value_to_path_buf(seg)?;
            let mut p = this.path.clone();
            p.push(seg);
            Ok(Self::new(p))
        });

        methods.add_method("dirname", |_, this, ()| {
            Ok(this
                .path
                .parent()
                .map_or_else(|| String::from("."), |p| p.to_string_lossy().into_owned()))
        });

        methods.add_method("basename", |_, this, ()| {
            Ok(this
                .path
                .file_name()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned()))
        });

        methods.add_method("extname", |_, this, ()| {
            Ok(this.path.extension().map(|s| s.to_string_lossy().into_owned()))
        });

        methods.add_method("stem", |_, this, ()| {
            Ok(this.path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        });

        methods.add_method("as_string", |_, this, ()| Ok(this.as_string()));
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.as_string()));
        methods.add_meta_method(MetaMethod::Eq, |_, a, b: AnyUserData| {
            let b = b.borrow::<Self>()?;
            Ok(a.path == b.path)
        });
    }
}

/// Initializes the `fs.path` module (pure path manipulation).
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "new",
        lua.create_function(|_, path: Value| Ok(PathObj::new(value_to_path_buf(path)?)))?,
    )?;

    t.set(
        "cwd",
        lua.create_function(|_, ()| {
            let cwd = std::env::current_dir().map_err(mlua::Error::external)?;
            Ok(PathObj::new(cwd))
        })?,
    )?;

    t.set(
        "join",
        lua.create_function(|_, (a, b): (Value, Value)| {
            let mut p = value_to_path_buf(a)?;
            p.push(value_to_path_buf(b)?);
            Ok(PathObj::new(p))
        })?,
    )?;

    Ok(t)
}

fn component_to_string(c: Component<'_>) -> String {
    match c {
        Component::Prefix(p) => p.as_os_str().to_string_lossy().into_owned(),
        Component::RootDir => String::from(std::path::MAIN_SEPARATOR),
        Component::CurDir => String::from("."),
        Component::ParentDir => String::from(".."),
        Component::Normal(s) => s.to_string_lossy().into_owned(),
    }
}

fn normalize_path(p: &Path) -> PathBuf {
    // Pure normalization, similar to "path-clean": remove '.' and fold '..' where possible.
    let mut out = PathBuf::new();
    let mut stack: Vec<Component<'_>> = Vec::new();

    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // Do not pop past a prefix/root. For relative paths, preserve leading "..".
                if let Some(last) = stack.last() {
                    match last {
                        Component::Normal(_) => {
                            stack.pop();
                        }
                        Component::ParentDir => {
                            stack.push(Component::ParentDir);
                        }
                        Component::RootDir | Component::Prefix(_) | Component::CurDir => {
                            // ignore (cannot go above root/prefix)
                        }
                    }
                } else {
                    stack.push(Component::ParentDir);
                }
            }
            other => stack.push(other),
        }
    }

    for c in stack {
        match c {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => out.push(".."),
            Component::Normal(s) => out.push(s),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn value_to_path_buf(value: Value) -> mlua::Result<PathBuf> {
    match value {
        Value::String(s) => Ok(PathBuf::from(s.to_str()?.to_owned())),
        Value::UserData(u) => {
            if let Ok(p) = u.borrow::<PathObj>() {
                Ok(p.path.clone())
            } else {
                Err(mlua::Error::external("expected path string or fs.path object"))
            }
        }
        other => Err(mlua::Error::external(format!(
            "expected path string or fs.path object, got {other:?}"
        ))),
    }
}
