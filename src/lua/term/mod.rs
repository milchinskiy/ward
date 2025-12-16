#![allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]

use mlua::{Lua, MetaMethod, MultiValue, Table, UserData, UserDataMethods, Value};
use std::io::{self, IsTerminal, Write};

/// Module init
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let term = lua.create_table()?;

    // Core interactive
    //
    // Supported call shapes:
    // - prompt("Question")
    // - prompt("Question", { default = "x", trim = true })
    // - prompt({ question = "Question", default = "x", trim = true })
    term.set(
        "prompt",
        lua.create_function(|lua, mv: MultiValue| {
            let args = parse_prompt_args(lua, mv)?;
            lua.create_userdata(InputAwaitable::prompt(args))
        })?,
    )?;

    // - confirm("Continue?")
    // - confirm("Continue?", { default = true })
    // - confirm({ question = "Continue?", default = true })
    term.set(
        "confirm",
        lua.create_function(|lua, mv: MultiValue| {
            let args = parse_confirm_args(lua, mv)?;
            lua.create_userdata(InputAwaitable::confirm(args))
        })?,
    )?;

    // - password("Password:")
    // - password("Password:", { trim = false })
    // - password({ prompt = "Password:", trim = false })
    term.set(
        "password",
        lua.create_function(|lua, mv: MultiValue| {
            let args = parse_password_args(lua, mv)?;
            lua.create_userdata(InputAwaitable::password(args))
        })?,
    )?;

    // - choose({ question = "Pick", choices = {"a", "b"}, default = 1 })
    term.set(
        "choose",
        lua.create_function(|lua, args: ChooseArgs| lua.create_userdata(InputAwaitable::choose(args)))?,
    )?;

    // Non-interactive helpers (synchronous)
    term.set(
        "print",
        lua.create_function(|_, v: Value| {
            write_value(false, v)?;
            Ok(true)
        })?,
    )?;
    term.set(
        "println",
        lua.create_function(|_, v: Option<Value>| {
            if let Some(v) = v {
                write_value(true, v)?;
            } else {
                println!();
            }
            Ok(true)
        })?,
    )?;
    term.set(
        "eprint",
        lua.create_function(|_, v: Value| {
            write_value_stderr(false, v)?;
            Ok(true)
        })?,
    )?;
    term.set(
        "eprintln",
        lua.create_function(|_, v: Option<Value>| {
            if let Some(v) = v {
                write_value_stderr(true, v)?;
            } else {
                eprintln!();
            }
            Ok(true)
        })?,
    )?;

    // ANSI helpers (works in most modern terminals; callers can decide when to use)
    term.set(
        "clear",
        lua.create_function(|_, ()| {
            // Clear screen + home cursor
            print!("\x1b[2J\x1b[H");
            io::stdout().flush().ok();
            Ok(true)
        })?,
    )?;

    term.set("ansi", lua.create_function(|_, spec: String| Ok(ansi_code(&spec)))?)?;

    term.set(
        "isatty",
        lua.create_function(|_, which: Option<String>| {
            let which = which.unwrap_or_else(|| "stdout".to_string());
            Ok(match which.to_ascii_lowercase().as_str() {
                "stdin" => io::stdin().is_terminal(),
                "stderr" => io::stderr().is_terminal(),
                _ => io::stdout().is_terminal(),
            })
        })?,
    )?;

    Ok(term)
}

// --- Argument parsing helpers ------------------------------------------------

fn parse_prompt_args(lua: &Lua, mut mv: MultiValue) -> mlua::Result<PromptArgs> {
    if mv.is_empty() {
        return Err(mlua::Error::external("prompt expects arguments"));
    }

    if mv.len() == 1 {
        return <PromptArgs as mlua::FromLua>::from_lua(mv.pop_front().unwrap(), lua);
    }

    // (string, table)
    let q = mv.pop_front().unwrap();
    let opts = mv.pop_front().unwrap();

    let Value::String(qs) = q else {
        return Err(mlua::Error::external("prompt first arg must be string"));
    };
    let question = qs.to_str()?.to_string();

    let Value::Table(t) = opts else {
        return Err(mlua::Error::external("prompt second arg must be table"));
    };

    Ok(PromptArgs {
        question,
        default: t.get::<Option<String>>("default")?,
        trim: t.get::<Option<bool>>("trim")?.unwrap_or(true),
    })
}

fn parse_confirm_args(lua: &Lua, mut mv: MultiValue) -> mlua::Result<ConfirmArgs> {
    if mv.is_empty() {
        return Err(mlua::Error::external("confirm expects arguments"));
    }

    if mv.len() == 1 {
        return <ConfirmArgs as mlua::FromLua>::from_lua(mv.pop_front().unwrap(), lua);
    }

    let q = mv.pop_front().unwrap();
    let opts = mv.pop_front().unwrap();

    let Value::String(qs) = q else {
        return Err(mlua::Error::external("confirm first arg must be string"));
    };
    let question = qs.to_str()?.to_string();

    let Value::Table(t) = opts else {
        return Err(mlua::Error::external("confirm second arg must be table"));
    };

    Ok(ConfirmArgs {
        question,
        default: t.get::<Option<bool>>("default")?,
    })
}

fn parse_password_args(lua: &Lua, mut mv: MultiValue) -> mlua::Result<PasswordArgs> {
    if mv.is_empty() {
        return Err(mlua::Error::external("password expects arguments"));
    }

    if mv.len() == 1 {
        return <PasswordArgs as mlua::FromLua>::from_lua(mv.pop_front().unwrap(), lua);
    }

    let p = mv.pop_front().unwrap();
    let opts = mv.pop_front().unwrap();

    let Value::String(ps) = p else {
        return Err(mlua::Error::external("password first arg must be string"));
    };
    let prompt = ps.to_str()?.to_string();

    let Value::Table(t) = opts else {
        return Err(mlua::Error::external("password second arg must be table"));
    };

    Ok(PasswordArgs {
        prompt,
        trim: t.get::<Option<bool>>("trim")?.unwrap_or(false),
    })
}

// --- Public argument types (Lua-friendly) -------------------------------------

/// term.prompt(question [, opts])
///
/// Lua forms supported:
/// - term.prompt("Name: ")
/// - term.prompt({ question = "Name: ", default = "", trim = true })
#[derive(Clone, Debug)]
pub struct PromptArgs {
    question: String,
    default: Option<String>,
    trim: bool,
}

impl mlua::FromLua for PromptArgs {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::String(s) => Ok(Self {
                question: s.to_str()?.to_string(),
                default: None,
                trim: true,
            }),
            Value::Table(t) => Ok(Self {
                question: t.get::<String>("question")?,
                default: t.get::<Option<String>>("default")?,
                trim: t.get::<Option<bool>>("trim")?.unwrap_or(true),
            }),
            _ => Err(mlua::Error::external(
                "prompt expects string or table {question, default?, trim?}",
            )),
        }
    }
}

/// term.confirm(question [, opts])
///
/// Lua forms supported:
/// - term.confirm("Continue?")
/// - term.confirm({ question = "Continue?", default = true })
#[derive(Clone, Debug)]
pub struct ConfirmArgs {
    question: String,
    default: Option<bool>,
}

impl mlua::FromLua for ConfirmArgs {
    fn from_lua(value: Value, _lua: &Lua) -> mlua::Result<Self> {
        match value {
            Value::String(s) => Ok(Self {
                question: s.to_str()?.to_string(),
                default: None,
            }),
            Value::Table(t) => Ok(Self {
                question: t.get::<String>("question")?,
                default: t.get::<Option<bool>>("default")?,
            }),
            _ => Err(mlua::Error::external("confirm expects string or table {question, default?}")),
        }
    }
}

/// term.password(prompt [, opts])
///
/// Lua forms supported:
/// - term.password("Password: ")
/// - term.password({ prompt = "Password: ", trim = false })
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct PasswordArgs {
    prompt: String,
    trim: bool,
}

impl mlua::FromLua for PasswordArgs {
    fn from_lua(value: Value, _lua: &Lua) -> mlua::Result<Self> {
        match value {
            Value::String(s) => Ok(Self {
                prompt: s.to_str()?.to_string(),
                trim: false,
            }),
            Value::Table(t) => Ok(Self {
                prompt: t.get::<String>("prompt")?,
                trim: t.get::<Option<bool>>("trim")?.unwrap_or(false),
            }),
            _ => Err(mlua::Error::external("password expects string or table {prompt, trim?}")),
        }
    }
}

/// term.choose({ question, choices, default? })
///
/// - choices: array-like table of strings
/// - returns: selected string (or nil on EOF)
#[derive(Clone, Debug)]
pub struct ChooseArgs {
    question: String,
    choices: Vec<String>,
    default_index: Option<usize>, // 1-based
}

impl mlua::FromLua for ChooseArgs {
    fn from_lua(value: Value, _lua: &Lua) -> mlua::Result<Self> {
        let Value::Table(t) = value else {
            return Err(mlua::Error::external("choose expects table {question, choices, default?}"));
        };

        let question = t.get::<String>("question")?;
        let choices_tbl: mlua::Table = t.get("choices")?;

        let mut choices = Vec::new();
        for pair in choices_tbl.sequence_values::<String>() {
            choices.push(pair?);
        }

        if choices.is_empty() {
            return Err(mlua::Error::external("choose requires non-empty choices"));
        }

        let default_index = t.get::<Option<usize>>("default")?;
        if let Some(i) = default_index
            && (i == 0 || i > choices.len())
        {
            return Err(mlua::Error::external("choose default is out of range"));
        }

        Ok(Self {
            question,
            choices,
            default_index,
        })
    }
}

// --- Awaitable implementation -------------------------------------------------

#[derive(Clone, Debug)]
enum InputKind {
    Prompt(PromptArgs),
    Confirm(ConfirmArgs),
    Password(PasswordArgs),
    Choose(ChooseArgs),
}

#[derive(Clone, Debug)]
struct InputAwaitable {
    kind: InputKind,
    consumed: bool,
}

impl InputAwaitable {
    fn prompt(args: PromptArgs) -> Self {
        Self {
            kind: InputKind::Prompt(args),
            consumed: false,
        }
    }

    fn confirm(args: ConfirmArgs) -> Self {
        Self {
            kind: InputKind::Confirm(args),
            consumed: false,
        }
    }

    fn password(args: PasswordArgs) -> Self {
        Self {
            kind: InputKind::Password(args),
            consumed: false,
        }
    }

    fn choose(args: ChooseArgs) -> Self {
        Self {
            kind: InputKind::Choose(args),
            consumed: false,
        }
    }
}

impl UserData for InputAwaitable {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // wait() -> value
        methods.add_async_method_mut("wait", |lua, mut this, ()| async move { input_wait(&lua, &mut this).await });

        // __call() -> value
        methods.add_async_meta_method_mut(MetaMethod::Call, |lua, mut this, ()| async move {
            input_wait(&lua, &mut this).await
        });

        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("InputAwaitable".to_string()));
    }
}

#[derive(Debug)]
enum InputResult {
    Str(String),
    Bool(bool),
    OptStr(Option<String>),
}

async fn input_wait(lua: &Lua, this: &mut InputAwaitable) -> mlua::Result<MultiValue> {
    if this.consumed {
        return Err(mlua::Error::external("input awaitable already consumed (create a new one)"));
    }
    this.consumed = true;

    // Move data into blocking closure
    let kind = this.kind.clone();

    let res: mlua::Result<InputResult> = tokio::task::spawn_blocking(move || match kind {
        InputKind::Prompt(args) => prompt_blocking(&args).map(InputResult::Str),
        InputKind::Confirm(args) => confirm_blocking(&args).map(InputResult::Bool),
        InputKind::Password(args) => password_blocking(&args).map(InputResult::Str),
        InputKind::Choose(args) => choose_blocking(&args).map(InputResult::OptStr),
    })
    .await
    .map_err(|e| mlua::Error::external(format!("input task join error: {e}")))?;

    match res? {
        InputResult::Str(s) => {
            let mut mv = MultiValue::new();
            mv.push_back(Value::String(lua.create_string(&s)?));
            Ok(mv)
        }
        InputResult::Bool(b) => {
            let mut mv = MultiValue::new();
            mv.push_back(Value::Boolean(b));
            Ok(mv)
        }
        InputResult::OptStr(opt) => {
            let mut mv = MultiValue::new();
            match opt {
                Some(s) => mv.push_back(Value::String(lua.create_string(&s)?)),
                None => mv.push_back(Value::Nil),
            }
            Ok(mv)
        }
    }
}

// --- Blocking IO primitives ---------------------------------------------------

fn prompt_blocking(args: &PromptArgs) -> mlua::Result<String> {
    let q = args.question.as_str();

    if let Some(def) = &args.default {
        print!("{q} [{def}] ");
    } else {
        print!("{q} ");
    }
    io::stdout().flush().ok();

    let mut buf = String::new();
    let n = io::stdin().read_line(&mut buf).map_err(mlua::Error::external)?;

    if n == 0 {
        // EOF
        return Ok(String::new());
    }

    if args.trim {
        let s = buf.trim_end_matches(['\r', '\n']).to_string();
        if s.is_empty() {
            Ok(args.default.clone().unwrap_or_default())
        } else {
            Ok(s)
        }
    } else if buf.is_empty() {
        Ok(args.default.clone().unwrap_or_default())
    } else {
        Ok(buf)
    }
}

fn confirm_blocking(args: &ConfirmArgs) -> mlua::Result<bool> {
    let suffix = match args.default {
        Some(true) => "[Y/n]",
        Some(false) | None => "[y/N]",
    };

    print!("{} {} ", args.question, suffix);
    io::stdout().flush().ok();

    let mut buf = String::new();
    let n = io::stdin().read_line(&mut buf).map_err(mlua::Error::external)?;

    if n == 0 {
        // EOF => treat as default or false
        return Ok(args.default.unwrap_or(false));
    }

    let s = buf.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Ok(args.default.unwrap_or(false));
    }

    // Accept [Yy](es)? | *
    Ok(s == "y" || s == "yes")
}

fn choose_blocking(args: &ChooseArgs) -> mlua::Result<Option<String>> {
    println!("{}", args.question);
    for (i, c) in args.choices.iter().enumerate() {
        println!("  {}) {}", i + 1, c);
    }

    let prompt = args.default_index.map_or_else(
        || format!("Select 1-{}: ", args.choices.len()),
        |d| format!("Select 1-{} [default {}]: ", args.choices.len(), d),
    );

    loop {
        print!("{prompt}");
        io::stdout().flush().ok();

        let mut buf = String::new();
        let n = io::stdin().read_line(&mut buf).map_err(mlua::Error::external)?;
        if n == 0 {
            return Ok(None);
        }

        let s = buf.trim();
        if s.is_empty() {
            if let Some(d) = args.default_index {
                return Ok(Some(args.choices[d - 1].clone()));
            }
            continue;
        }

        if let Ok(i) = s.parse::<usize>()
            && i >= 1
            && i <= args.choices.len()
        {
            return Ok(Some(args.choices[i - 1].clone()));
        }

        // reprompt
    }
}

fn password_blocking(args: &PasswordArgs) -> mlua::Result<String> {
    // Reads from the controlling tty with echo disabled
    let s = rpassword::read_password().map_err(mlua::Error::external)?;
    if args.trim {
        Ok(s.trim_end_matches(['\r', '\n']).to_string())
    } else {
        Ok(s)
    }
}

// --- Printing helpers ---------------------------------------------------------

fn write_value(newline: bool, v: Value) -> mlua::Result<()> {
    let s = match v {
        Value::String(s) => s.to_str()?.to_string(),
        other => format!("{other:?}"),
    };
    if newline {
        println!("{s}");
    } else {
        print!("{s}");
        io::stdout().flush().ok();
    }
    Ok(())
}

fn write_value_stderr(newline: bool, v: Value) -> mlua::Result<()> {
    let s = match v {
        Value::String(s) => s.to_str()?.to_string(),
        other => format!("{other:?}"),
    };
    if newline {
        eprintln!("{s}");
    } else {
        eprint!("{s}");
        io::stderr().flush().ok();
    }
    Ok(())
}

/// Returns ANSI escape code by spec (e.g., "reset", "bold", "red", "green", "yellow", "blue").
/// This is intentionally minimal; expand as needed.
fn ansi_code(spec: &str) -> String {
    match spec.to_ascii_lowercase().as_str() {
        "reset" => "\x1b[0m".to_string(),
        "bold" => "\x1b[1m".to_string(),
        "dim" => "\x1b[2m".to_string(),
        "underline" => "\x1b[4m".to_string(),
        "red" => "\x1b[31m".to_string(),
        "green" => "\x1b[32m".to_string(),
        "yellow" => "\x1b[33m".to_string(),
        "blue" => "\x1b[34m".to_string(),
        "magenta" => "\x1b[35m".to_string(),
        "cyan" => "\x1b[36m".to_string(),
        "gray" | "grey" => "\x1b[90m".to_string(),
        _ => String::new(),
    }
}
