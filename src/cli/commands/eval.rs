use rust_args_parser as ap;
use tokio::task::LocalSet;

use super::common::{build_runtime, parse_timeout_arg, prepare_sandbox};
use ward::runner::DEFAULT_THREAD_POOL_SIZE;

#[derive(Default, Debug)]
pub struct EvalContext {
    expr: Option<String>,
    args: Vec<std::ffi::OsString>,
    memory_limit: Option<usize>,
    instruction_limit: Option<u64>,
    thread_pool_size: Option<usize>,
    timeout: Option<std::time::Duration>,
    no_stdin: bool,
}

#[allow(clippy::too_many_lines)]
pub fn command<'a>() -> ap::CmdSpec<'a, super::Context> {
    ap::CmdSpec::new("eval")
        .help("Evaluate Lua code")
        .opt(
            ap::OptSpec::value("expr", |value, ctx: &mut super::Context| {
                ctx.eval.expr = Some(value.to_string_lossy().into_owned());
                Ok(())
            })
            .long("expr")
            .short('e')
            .help("Lua code to evaluate")
            .single(),
        )
        .opt(
            ap::OptSpec::flag("no-stdin", |ctx: &mut super::Context| {
                ctx.eval.no_stdin = true;
                Ok(())
            })
            .long("no-stdin")
            .help("Do not read code from stdin when --expr is not provided")
            .single(),
        )
        .pos(
            ap::PosSpec::new("ARG", |value, ctx: &mut super::Context| {
                ctx.eval.args.push(value.to_os_string());
                Ok(())
            })
            .help("Arguments forwarded to the evaluated chunk (use -- to terminate CLI parsing)")
            .range(0, usize::MAX),
        )
        .opt(
            ap::OptSpec::value("memory-limit", |value, ctx: &mut super::Context| {
                let v: usize = value.to_string_lossy().parse().map_err(ap::Error::user)?;
                if v == 0 {
                    return Err(ap::Error::User("Memory limit must be greater than 0".into()));
                }
                ctx.eval.memory_limit = Some(v);
                Ok(())
            })
            .long("memory-limit")
            .short('m')
            .help("Memory limit in bytes")
            .single(),
        )
        .opt(
            ap::OptSpec::value("instruction-limit", |value, ctx: &mut super::Context| {
                let v: u64 = value.to_string_lossy().parse().map_err(ap::Error::user)?;
                if v == 0 {
                    return Err(ap::Error::User("Instruction limit must be greater than 0".into()));
                }
                ctx.eval.instruction_limit = Some(v);
                Ok(())
            })
            .long("instruction-limit")
            .short('i')
            .help("Instruction limit (approximate; enforced via VM hook every ~1024 instructions)")
            .single(),
        )
        .opt(
            ap::OptSpec::value("threads", |value, ctx: &mut super::Context| {
                let v: usize = value.to_string_lossy().parse().map_err(ap::Error::user)?;
                if v == 0 {
                    return Err(ap::Error::User("Thread pool size must be greater than 0".into()));
                }
                ctx.eval.thread_pool_size = Some(v);
                Ok(())
            })
            .long("threads")
            .short('t')
            .help("Thread pool size")
            .single(),
        )
        .opt(
            ap::OptSpec::value("timeout", |value, ctx: &mut super::Context| {
                let dur = parse_timeout_arg(value)?;
                ctx.eval.timeout = Some(dur);
                Ok(())
            })
            .long("timeout")
            .short('T')
            .help("Timeout (accepts seconds or duration strings like 500ms, 2s, 1m)")
            .single(),
        )
        .handler(|_, ctx: &mut super::Context| {
            let code = if let Some(ref expr) = ctx.eval.expr {
                expr.clone()
            } else if ctx.eval.no_stdin {
                return Err(ap::Error::User("no code provided (use --expr or pipe into stdin)".into()));
            } else {
                read_stdin_to_string().map_err(ap::Error::user)?
            };

            let runtime = build_runtime(ctx.eval.thread_pool_size.unwrap_or(DEFAULT_THREAD_POOL_SIZE))?;
            let sandbox = prepare_sandbox(
                ctx.eval.memory_limit,
                ctx.eval.instruction_limit,
                ctx.eval.thread_pool_size,
                ctx.eval.timeout,
            );

            let local = LocalSet::new();
            match local.block_on(&runtime, ward::runner::eval(code.as_str(), &ctx.eval.args, sandbox)) {
                Ok(()) => Ok(()),
                Err(ward::Error::Exit(code)) => std::process::exit(code),
                Err(e) => Err(ap::Error::user(e)),
            }
        })
}

fn read_stdin_to_string() -> std::io::Result<String> {
    use std::io::Read;

    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}
