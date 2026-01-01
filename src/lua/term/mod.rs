#![allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]

use mlua::{Lua, MetaMethod, MultiValue, Table, UserData, UserDataMethods, Value};
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

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
        lua.create_function(|lua, mv: MultiValue| {
            write_values_stdout(lua, false, mv)?;
            Ok(true)
        })?,
    )?;
    term.set(
        "println",
        lua.create_function(|lua, mv: MultiValue| {
            if mv.is_empty() {
                println!();
            } else {
                write_values_stdout(lua, true, mv)?;
            }
            Ok(true)
        })?,
    )?;
    term.set(
        "eprint",
        lua.create_function(|lua, mv: MultiValue| {
            write_values_stderr(lua, false, mv)?;
            Ok(true)
        })?,
    )?;
    term.set(
        "eprintln",
        lua.create_function(|lua, mv: MultiValue| {
            if mv.is_empty() {
                eprintln!();
            } else {
                write_values_stderr(lua, true, mv)?;
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

    // ANSI escape codes as constants (shell-like ergonomics): term.ansi.red .. "text" .. term.ansi.reset
    term.set("ansi", ansi_table(lua)?)?;

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

    // Progress indicator for interactive scripts.
    // Usage:
    //   local p = term.progress({ total = 100, message = "Downloading", stream = "stderr" })
    //   p:tick() ... p:finish()
    term.set(
        "progress",
        lua.create_function(|lua, mv: MultiValue| {
            let args = parse_progress_args(mv)?;
            lua.create_userdata(Progress::new(args))
        })?,
    )?;

    Ok(term)
}

fn parse_prompt_args(lua: &Lua, mut mv: MultiValue) -> mlua::Result<PromptArgs> {
    if mv.is_empty() {
        return Err(mlua::Error::external("prompt expects arguments"));
    }

    if mv.len() == 1 {
        let Some(q) = mv.pop_front() else {
            return Err(mlua::Error::external("Error parsing prompt"));
        };
        return <PromptArgs as mlua::FromLua>::from_lua(q, lua);
    }

    // (string, table)
    let Some(q) = mv.pop_front() else {
        return Err(mlua::Error::external("Error parsing prompt"));
    };
    let Some(opts) = mv.pop_front() else {
        return Err(mlua::Error::external("Error parsing prompt"));
    };

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
        let Some(q) = mv.pop_front() else {
            return Err(mlua::Error::external("Error parsing prompt"));
        };
        return <ConfirmArgs as mlua::FromLua>::from_lua(q, lua);
    }

    let Some(q) = mv.pop_front() else {
        return Err(mlua::Error::external("Error parsing prompt"));
    };
    let Some(opts) = mv.pop_front() else {
        return Err(mlua::Error::external("Error parsing prompt"));
    };

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
        let Some(q) = mv.pop_front() else {
            return Err(mlua::Error::external("Error parsing prompt"));
        };
        return <PasswordArgs as mlua::FromLua>::from_lua(q, lua);
    }

    let Some(p) = mv.pop_front() else {
        return Err(mlua::Error::external("Error parsing prompt"));
    };
    let Some(opts) = mv.pop_front() else {
        return Err(mlua::Error::external("Error parsing prompt"));
    };

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
}

impl InputAwaitable {
    fn prompt(args: PromptArgs) -> Self {
        Self {
            kind: InputKind::Prompt(args),
        }
    }

    fn confirm(args: ConfirmArgs) -> Self {
        Self {
            kind: InputKind::Confirm(args),
        }
    }

    fn password(args: PasswordArgs) -> Self {
        Self {
            kind: InputKind::Password(args),
        }
    }

    fn choose(args: ChooseArgs) -> Self {
        Self {
            kind: InputKind::Choose(args),
        }
    }
}

impl UserData for InputAwaitable {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // wait() -> value
        methods.add_async_method_mut("wait", |lua, this, ()| async move { input_wait(&lua, &this).await });

        // __call() -> value
        methods
            .add_async_meta_method_mut(MetaMethod::Call, |lua, this, ()| async move { input_wait(&lua, &this).await });

        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("InputAwaitable".to_string()));
    }
}

#[derive(Debug)]
enum InputResult {
    Bool(bool),
    OptStr(Option<String>),
}

async fn input_wait(lua: &Lua, this: &InputAwaitable) -> mlua::Result<MultiValue> {
    let console = super::console::console(lua);
    let _guard = console.interactive.lock().await;

    let res: InputResult = match this.kind.clone() {
        InputKind::Prompt(args) => InputResult::OptStr(prompt_async(&console, &args).await?),
        InputKind::Confirm(args) => InputResult::Bool(confirm_async(&console, &args).await?),
        InputKind::Choose(args) => InputResult::OptStr(choose_async(&console, &args).await?),
        InputKind::Password(args) => InputResult::OptStr(password_async(&console, &args).await?),
    };

    match res {
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

async fn stdout_write(console: &Arc<crate::lua::console::Console>, s: &str) -> mlua::Result<()> {
    let mut out = console.stdout.lock().await;
    out.write_all(s.as_bytes()).await.map_err(mlua::Error::external)?;
    out.flush().await.map_err(mlua::Error::external)?;
    drop(out);
    Ok(())
}

async fn prompt_async(console: &Arc<crate::lua::console::Console>, args: &PromptArgs) -> mlua::Result<Option<String>> {
    let q = args.question.as_str();
    let prompt = args
        .default
        .as_ref()
        .map_or_else(|| format!("{q} "), |def| format!("{q} [{def}] "));
    stdout_write(console, &prompt).await?;

    let mut buf = String::new();
    let bytes = {
        let mut stdin = console.stdin.lock().await;
        stdin.read_line(&mut buf).await.map_err(mlua::Error::external)?
    };

    if bytes == 0 {
        return Ok(None);
    }

    if args.trim {
        let s = buf.trim_end_matches(['\r', '\n']).to_string();
        if s.is_empty() {
            Ok(Some(args.default.clone().unwrap_or_default()))
        } else {
            Ok(Some(s))
        }
    } else if buf.is_empty() {
        Ok(Some(args.default.clone().unwrap_or_default()))
    } else {
        Ok(Some(buf))
    }
}

async fn confirm_async(console: &Arc<crate::lua::console::Console>, args: &ConfirmArgs) -> mlua::Result<bool> {
    let suffix = match args.default {
        Some(true) => "[Y/n]",
        Some(false) | None => "[y/N]",
    };

    stdout_write(console, &format!("{} {} ", args.question, suffix)).await?;
    let mut buf = String::new();
    let bytes = {
        let mut stdin = console.stdin.lock().await;
        stdin.read_line(&mut buf).await.map_err(mlua::Error::external)?
    };
    if bytes == 0 {
        return Ok(args.default.unwrap_or(false));
    }
    let s = buf.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Ok(args.default.unwrap_or(false));
    }

    Ok(s == "y" || s == "yes")
}

async fn choose_async(console: &Arc<crate::lua::console::Console>, args: &ChooseArgs) -> mlua::Result<Option<String>> {
    let mut menu = String::new();
    menu.push_str(&args.question);
    menu.push('\n');
    for (i, c) in args.choices.iter().enumerate() {
        let s = format!("  {}) {}\n", i + 1, c);
        menu.push_str(s.as_str());
    }
    stdout_write(console, &menu).await?;
    let prompt = args.default_index.map_or_else(
        || format!("Select 1-{}: ", args.choices.len()),
        |d| format!("Select 1-{} [default {}]: ", args.choices.len(), d),
    );

    loop {
        stdout_write(console, &prompt).await?;

        let mut buf = String::new();
        let bytes = {
            let mut stdin = console.stdin.lock().await;
            stdin.read_line(&mut buf).await.map_err(mlua::Error::external)?
        };

        if bytes == 0 {
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
    }
}

async fn password_async(
    console: &Arc<crate::lua::console::Console>,
    args: &PasswordArgs,
) -> mlua::Result<Option<String>> {
    stdout_write(console, &format!("{} ", args.prompt)).await?;
    let res = tokio::task::spawn_blocking(rpassword::read_password)
        .await
        .map_err(|e| mlua::Error::external(format!("password task join error: {e}")))?;

    match res {
        Ok(s) => {
            if args.trim {
                Ok(Some(s.trim_end_matches(['\r', '\n']).to_string()))
            } else {
                Ok(Some(s))
            }
        }
        Err(e) => {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                Ok(None)
            } else {
                Err(mlua::Error::external(e))
            }
        }
    }
}

fn join_values(lua: &Lua, mv: MultiValue) -> mlua::Result<String> {
    // Match Lua's print() convention: tostring each value and join with tabs.
    let tostring: mlua::Function = lua.globals().get("tostring")?;
    let mut out = String::new();
    for (i, v) in mv.into_iter().enumerate() {
        if i > 0 {
            out.push('\t');
        }
        let s: String = tostring.call(v)?;
        out.push_str(&s);
    }
    Ok(out)
}

fn write_values_stdout(lua: &Lua, newline: bool, mv: MultiValue) -> mlua::Result<()> {
    if mv.is_empty() {
        return Ok(());
    }
    let s = join_values(lua, mv)?;
    if newline {
        println!("{s}");
    } else {
        print!("{s}");
        io::stdout().flush().ok();
    }
    Ok(())
}

fn write_values_stderr(lua: &Lua, newline: bool, mv: MultiValue) -> mlua::Result<()> {
    if mv.is_empty() {
        return Ok(());
    }
    let s = join_values(lua, mv)?;
    if newline {
        eprintln!("{s}");
    } else {
        eprint!("{s}");
        io::stderr().flush().ok();
    }
    Ok(())
}

// --- ANSI submodule ----------------------------------------------------------

fn ansi_table(lua: &Lua) -> mlua::Result<Table> {
    let ansi = lua.create_table()?;

    // Styles
    ansi.set("reset", "\x1b[0m")?;
    ansi.set("bold", "\x1b[1m")?;
    ansi.set("dim", "\x1b[2m")?;
    ansi.set("italic", "\x1b[3m")?;
    ansi.set("underline", "\x1b[4m")?;
    ansi.set("blink", "\x1b[5m")?;
    ansi.set("reverse", "\x1b[7m")?;
    ansi.set("hidden", "\x1b[8m")?;
    ansi.set("strike", "\x1b[9m")?;

    // Cursor / clearing helpers
    ansi.set("clear_line", "\x1b[2K")?;
    ansi.set("clear_screen", "\x1b[2J")?;
    ansi.set("home", "\x1b[H")?;

    // Foreground colors
    ansi.set("black", "\x1b[30m")?;
    ansi.set("red", "\x1b[31m")?;
    ansi.set("green", "\x1b[32m")?;
    ansi.set("yellow", "\x1b[33m")?;
    ansi.set("blue", "\x1b[34m")?;
    ansi.set("magenta", "\x1b[35m")?;
    ansi.set("cyan", "\x1b[36m")?;
    ansi.set("white", "\x1b[37m")?;
    ansi.set("default", "\x1b[39m")?;

    // Bright foreground colors
    ansi.set("bright_black", "\x1b[90m")?;
    ansi.set("bright_red", "\x1b[91m")?;
    ansi.set("bright_green", "\x1b[92m")?;
    ansi.set("bright_yellow", "\x1b[93m")?;
    ansi.set("bright_blue", "\x1b[94m")?;
    ansi.set("bright_magenta", "\x1b[95m")?;
    ansi.set("bright_cyan", "\x1b[96m")?;
    ansi.set("bright_white", "\x1b[97m")?;

    // Background colors
    ansi.set("bg_black", "\x1b[40m")?;
    ansi.set("bg_red", "\x1b[41m")?;
    ansi.set("bg_green", "\x1b[42m")?;
    ansi.set("bg_yellow", "\x1b[43m")?;
    ansi.set("bg_blue", "\x1b[44m")?;
    ansi.set("bg_magenta", "\x1b[45m")?;
    ansi.set("bg_cyan", "\x1b[46m")?;
    ansi.set("bg_white", "\x1b[47m")?;
    ansi.set("bg_default", "\x1b[49m")?;

    // Bright background colors
    ansi.set("bg_bright_black", "\x1b[100m")?;
    ansi.set("bg_bright_red", "\x1b[101m")?;
    ansi.set("bg_bright_green", "\x1b[102m")?;
    ansi.set("bg_bright_yellow", "\x1b[103m")?;
    ansi.set("bg_bright_blue", "\x1b[104m")?;
    ansi.set("bg_bright_magenta", "\x1b[105m")?;
    ansi.set("bg_bright_cyan", "\x1b[106m")?;
    ansi.set("bg_bright_white", "\x1b[107m")?;

    Ok(ansi)
}

// --- Progress indicator ------------------------------------------------------

#[derive(Clone, Debug)]
enum ProgressStream {
    Stdout,
    Stderr,
}

impl ProgressStream {
    fn is_tty(&self) -> bool {
        match self {
            Self::Stdout => io::stdout().is_terminal(),
            Self::Stderr => io::stderr().is_terminal(),
        }
    }

    fn write_all(&self, s: &str) {
        match self {
            Self::Stdout => {
                print!("{s}");
                io::stdout().flush().ok();
            }
            Self::Stderr => {
                eprint!("{s}");
                io::stderr().flush().ok();
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ProgressArgs {
    total: Option<u64>,
    message: Option<String>,
    width: usize,
    stream: ProgressStream,
}

fn parse_progress_args(mut mv: MultiValue) -> mlua::Result<ProgressArgs> {
    if mv.is_empty() {
        return Ok(ProgressArgs {
            total: None,
            message: None,
            width: 40,
            stream: ProgressStream::Stderr,
        });
    }

    if mv.len() == 1 {
        let Some(q) = mv.pop_front() else {
            return Err(mlua::Error::external("Error parsing prompt"));
        };
        match q {
            Value::Integer(i) => {
                let total = u64::try_from(i.max(0)).map_err(mlua::Error::external)?;
                return Ok(ProgressArgs {
                    total: Some(total),
                    message: None,
                    width: 40,
                    stream: ProgressStream::Stderr,
                });
            }
            Value::Number(n) => {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let total = n.max(0.0) as u64;
                return Ok(ProgressArgs {
                    total: Some(total),
                    message: None,
                    width: 40,
                    stream: ProgressStream::Stderr,
                });
            }
            Value::Table(t) => {
                let total = match t.get::<Option<Value>>("total")? {
                    Some(Value::Integer(i)) => Some(u64::try_from(i.max(0)).map_err(mlua::Error::external)?),
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    Some(Value::Number(n)) => Some(n.max(0.0) as u64),
                    _ => None,
                };

                let message = t.get::<Option<String>>("message")?;
                let width = t.get::<Option<usize>>("width")?.unwrap_or(40).max(10);
                let stream = match t
                    .get::<Option<String>>("stream")?
                    .unwrap_or_else(|| "stderr".to_string())
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "stdout" => ProgressStream::Stdout,
                    _ => ProgressStream::Stderr,
                };

                return Ok(ProgressArgs {
                    total,
                    message,
                    width,
                    stream,
                });
            }
            other => {
                return Err(mlua::Error::external(format!(
                    "progress expects a table or a number, got {other:?}"
                )));
            }
        }
    }

    Err(mlua::Error::external("progress expects at most one argument"))
}

#[derive(Clone, Debug)]
struct Progress {
    total: Option<u64>,
    current: u64,
    message: Option<String>,
    width: usize,
    stream: ProgressStream,
    enabled: bool,
    started: std::time::Instant,
    last_render: std::time::Instant,
    spinner_idx: usize,
    finished: bool,
}

impl Progress {
    fn new(args: ProgressArgs) -> Self {
        let now = std::time::Instant::now();
        let enabled = args.stream.is_tty();
        Self {
            total: args.total,
            current: 0,
            message: args.message,
            width: args.width,
            stream: args.stream,
            enabled,
            started: now,
            last_render: now,
            spinner_idx: 0,
            finished: false,
        }
    }

    fn render(&mut self, force: bool) {
        if self.finished {
            return;
        }

        // Avoid excessive redraw.
        let now = std::time::Instant::now();
        if !force && now.duration_since(self.last_render) < std::time::Duration::from_millis(50) {
            return;
        }
        self.last_render = now;

        if !self.enabled {
            return;
        }

        let elapsed = now.duration_since(self.started);
        let secs = elapsed.as_secs();

        let prefix = self.message.as_deref().unwrap_or("");

        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let line = if let Some(total) = self.total {
            let total = total.max(1);
            let cur = self.current.min(total);
            let ratio = (cur as f64) / (total as f64);
            let filled = ((ratio * (self.width as f64)).round() as usize).min(self.width);
            let empty = self.width.saturating_sub(filled);
            let pct = (ratio * 100.0).round() as u64;

            format!(
                "\r\x1b[2K{prefix}[{}{}] {}/{} ({}%) {}s",
                "#".repeat(filled),
                "-".repeat(empty),
                cur,
                total,
                pct,
                secs
            )
        } else {
            let frames = ['|', '/', '-', '\\'];
            let ch = frames[self.spinner_idx % frames.len()];
            self.spinner_idx = self.spinner_idx.wrapping_add(1);
            format!("\r\x1b[2K{prefix}{ch} {secs}s")
        };

        self.stream.write_all(&line);
    }

    fn finish(&mut self, msg: Option<String>) {
        if self.finished {
            return;
        }

        if let Some(m) = msg {
            self.message = Some(m);
        }
        self.render(true);

        if self.enabled {
            self.stream.write_all("\n");
        }
        self.finished = true;
    }
}

impl UserData for Progress {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("tick", |_, this, delta: Option<i64>| {
            #[allow(clippy::cast_sign_loss)]
            let d = delta.unwrap_or(1).max(0) as u64;
            this.current = this.current.saturating_add(d);
            // Force render when total reached.
            let force = this.total.is_some_and(|t| t > 0 && this.current >= t);
            this.render(force);
            Ok(Value::Nil)
        });

        methods.add_method_mut("value", |_, this, value: Option<i64>| {
            if let Some(value) = value {
                this.current = u64::try_from(value.max(0)).map_err(mlua::Error::external)?;
                this.render(false);
                Ok(Value::Nil)
            } else {
                #[allow(clippy::cast_possible_wrap)]
                Ok(Value::Integer(this.current as i64))
            }
        });

        methods.add_method_mut("total", |_, this, total: Option<i64>| {
            if let Some(total) = total {
                this.total = Some(u64::try_from(total.max(0)).map_err(mlua::Error::external)?);
                this.render(true);
                Ok(Value::Nil)
            } else {
                #[allow(clippy::cast_possible_wrap)]
                Ok(Value::Integer(this.total.unwrap_or(0) as i64))
            }
        });

        methods.add_method_mut("message", |lua, this, msg: Option<String>| {
            if let Some(msg) = msg {
                this.message = Some(msg);
                this.render(true);
                return Ok(Value::Nil);
            }

            let msg = this.message.clone().unwrap_or(String::new());
            Ok(Value::String(lua.create_string(&msg)?))
        });

        methods.add_method_mut("finish", |_, this, msg: Option<String>| {
            this.finish(msg);
            Ok(true)
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Progress(total={:?}, current={})", this.total, this.current))
        });
    }
}
