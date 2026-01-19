use rust_args_parser as ap;
use tokio::task::LocalSet;

use super::common::{build_runtime, parse_timeout_arg, prepare_sandbox};
use ward::runner::DEFAULT_THREAD_POOL_SIZE;

#[derive(Default, Debug)]
pub struct ReplContext {
    args: Vec<std::ffi::OsString>,
    memory_limit: Option<usize>,
    instruction_limit: Option<u64>,
    thread_pool_size: Option<usize>,
    timeout: Option<std::time::Duration>,
    no_prompt: bool,
}

pub fn command<'a>() -> ap::CmdSpec<'a, super::Context> {
    ap::CmdSpec::new("repl")
        .help("Start an interactive Ward Lua REPL")
        .opt(
            ap::OptSpec::flag("no-prompt", |ctx: &mut super::Context| {
                ctx.repl.no_prompt = true;
                Ok(())
            })
            .long("no-prompt")
            .help("Disable prompts and banner")
            .single(),
        )
        .pos(
            ap::PosSpec::new("ARG", |value, ctx: &mut super::Context| {
                ctx.repl.args.push(value.to_os_string());
                Ok(())
            })
            .help("Arguments forwarded to the REPL session (use -- to terminate CLI parsing)")
            .range(0, usize::MAX),
        )
        .opt(
            ap::OptSpec::value("memory-limit", |value, ctx: &mut super::Context| {
                let v: usize = value.to_string_lossy().parse().map_err(ap::Error::user)?;
                if v == 0 {
                    return Err(ap::Error::User("Memory limit must be greater than 0".into()));
                }
                ctx.repl.memory_limit = Some(v);
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
                ctx.repl.instruction_limit = Some(v);
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
                ctx.repl.thread_pool_size = Some(v);
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
                ctx.repl.timeout = Some(dur);
                Ok(())
            })
            .long("timeout")
            .short('T')
            .help("Timeout (accepts seconds or duration strings like 500ms, 2s, 1m)")
            .single(),
        )
        .handler(|_, ctx: &mut super::Context| {
            let runtime = build_runtime(ctx.repl.thread_pool_size.unwrap_or(DEFAULT_THREAD_POOL_SIZE))?;
            let sandbox = prepare_sandbox(
                ctx.repl.memory_limit,
                ctx.repl.instruction_limit,
                ctx.repl.thread_pool_size,
                ctx.repl.timeout,
            );

            let local = LocalSet::new();
            match local.block_on(&runtime, ward::runner::repl(&ctx.repl.args, sandbox, ctx.repl.no_prompt)) {
                Ok(()) => Ok(()),
                Err(ward::Error::Exit(code)) => std::process::exit(code),
                Err(e) => Err(ap::Error::user(e)),
            }
        })
}
