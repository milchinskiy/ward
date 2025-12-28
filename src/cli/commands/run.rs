use rust_args_parser as ap;
use serde_ext_duration::parse_str as parse_duration_str;
use tokio::task::LocalSet;

#[derive(Default, Debug)]
pub struct RunContext {
    file: std::path::PathBuf,
    args: Vec<std::ffi::OsString>,
    memory_limit: Option<usize>,
    instruction_limit: Option<u64>,
    thread_pool_size: Option<usize>,
    timeout: Option<std::time::Duration>,
}

fn parse_timeout_arg(value: &std::ffi::OsStr) -> Result<std::time::Duration, ap::Error> {
    let lossy = value.to_string_lossy();
    if let Ok(dur) = parse_duration_str(&lossy) {
        return Ok(dur);
    }
    let timeout: f64 = lossy.parse().map_err(ap::Error::user)?;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )]
    let timeout: u64 = if timeout > u64::MAX as f64 {
        u64::MAX
    } else {
        (timeout * 1000.0) as u64
    };
    Ok(std::time::Duration::from_millis(timeout))
}

pub fn command<'a>() -> ap::CmdSpec<'a, super::Context> {
    ap::CmdSpec::new("run")
        .help("Run a lua file")
        .pos(
            ap::PosSpec::new("FILE", |value, ctx: &mut super::Context| {
                if ctx.run.file.as_os_str().is_empty() {
                    ctx.run.file = std::path::PathBuf::from(value);
                } else {
                    ctx.run.args.push(value.to_os_string());
                }
                Ok(())
            })
            .help("Lua file to run (extra values are forwarded to the script; use -- to terminate CLI parsing)")
            .range(1, usize::MAX),
        )
        .opt(
            ap::OptSpec::value("memory-limit", |value, ctx: &mut super::Context| {
                let v: usize = value.to_string_lossy().parse().map_err(ap::Error::user)?;
                if v == 0 {
                    return Err(ap::Error::User("Memory limit must be greater than 0".into()));
                }
                ctx.run.memory_limit = Some(v);
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
                ctx.run.instruction_limit = Some(v);
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
                ctx.run.thread_pool_size = Some(v);
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
                ctx.run.timeout = Some(dur);
                Ok(())
            })
            .long("timeout")
            .short('T')
            .help("Timeout (accepts seconds or duration strings like 500ms, 2s, 1m)")
            .single(),
        )
        .handler(|_, ctx: &mut super::Context| {
            let workers = ctx.run.thread_pool_size.unwrap_or(2);
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(workers)
                .max_blocking_threads(workers)
                .enable_all()
                .build()
                .map_err(ap::Error::user)?;

            let sandbox_default = ward::runner::sandbox::SandboxPolicy::default();
            let sandbox = ward::runner::sandbox::SandboxPolicy {
                memory_limit_bytes: ctx.run.memory_limit.unwrap_or(sandbox_default.memory_limit_bytes),
                instruction_limit: ctx.run.instruction_limit.unwrap_or(sandbox_default.instruction_limit),
                thread_pool_size: ctx.run.thread_pool_size.unwrap_or(sandbox_default.thread_pool_size),
                timeout: ctx.run.timeout,
            };
            let local = LocalSet::new();
            match local.block_on(&runtime, ward::runner::run_file(ctx.run.file.as_path(), &ctx.run.args, sandbox)) {
                Ok(()) => Ok(()),
                Err(ward::Error::Exit(code)) => std::process::exit(code),
                Err(e) => Err(ap::Error::user(e)),
            }
        })
}
