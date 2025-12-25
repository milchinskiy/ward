use rust_args_parser as ap;
use tokio::task::LocalSet;

#[derive(Default, Debug)]
pub struct RunContext {
    file: std::path::PathBuf,
    memory_limit: Option<usize>,
    instruction_limit: Option<u64>,
    thread_pool_size: Option<usize>,
    timeout: Option<std::time::Duration>,
}

pub fn command<'a>() -> ap::CmdSpec<'a, super::Context> {
    ap::CmdSpec::new("run")
        .help("Run a lua file")
        .pos(
            ap::PosSpec::new("FILE", |value, ctx: &mut super::Context| {
                ctx.run.file = std::path::PathBuf::from(value);
                Ok(())
            })
            .help("Lua file to run")
            .required(),
        )
        .opt(
            ap::OptSpec::value("memory-limit", |value, ctx: &mut super::Context| {
                ctx.run.memory_limit = Some(value.to_string_lossy().parse().map_err(ap::Error::user)?);
                Ok(())
            })
            .long("memory-limit")
            .short('m')
            .help("Memory limit in bytes")
            .single(),
        )
        .opt(
            ap::OptSpec::value("instruction-limit", |value, ctx: &mut super::Context| {
                ctx.run.instruction_limit = Some(value.to_string_lossy().parse().map_err(ap::Error::user)?);
                Ok(())
            })
            .long("instruction-limit")
            .short('i')
            .help("Instruction limit")
            .single(),
        )
        .opt(
            ap::OptSpec::value("threads", |value, ctx: &mut super::Context| {
                ctx.run.thread_pool_size = Some(value.to_string_lossy().parse().map_err(ap::Error::user)?);
                Ok(())
            })
            .long("threads")
            .short('t')
            .help("Thread pool size")
            .single(),
        )
        .opt(
            ap::OptSpec::value("timeout", |value, ctx: &mut super::Context| {
                let timeout: f64 = value.to_string_lossy().parse().map_err(ap::Error::user)?;
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
                ctx.run.timeout = Some(std::time::Duration::from_millis(timeout));
                Ok(())
            })
            .long("timeout")
            .short('T')
            .help("Timeout in seconds")
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
            local
                .block_on(&runtime, ward::runner::run_file(ctx.run.file.as_path(), sandbox))
                .map_err(ap::Error::user)
        })
}
