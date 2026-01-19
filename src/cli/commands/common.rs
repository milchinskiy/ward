use rust_args_parser as ap;
use serde_ext_duration::parse_str as parse_duration_str;

pub(super) fn parse_timeout_arg(value: &std::ffi::OsStr) -> Result<std::time::Duration, ap::Error> {
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
    let timeout_ms: u64 = if timeout > u64::MAX as f64 {
        u64::MAX
    } else {
        (timeout * 1000.0) as u64
    };

    Ok(std::time::Duration::from_millis(timeout_ms))
}

pub(super) fn build_runtime(workers: usize) -> Result<tokio::runtime::Runtime, ap::Error> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .max_blocking_threads(workers)
        .enable_all()
        .build()
        .map_err(ap::Error::user)
}

pub(super) fn prepare_sandbox(
    memory_limit: Option<usize>,
    instruction_limit: Option<u64>,
    thread_pool_size: Option<usize>,
    timeout: Option<std::time::Duration>,
) -> ward::runner::sandbox::SandboxPolicy {
    let mut sandbox = ward::runner::sandbox::SandboxPolicy::default();

    if let Some(mem) = memory_limit {
        sandbox.memory_limit_bytes = mem;
    }
    if let Some(inst) = instruction_limit {
        sandbox.instruction_limit = inst;
    }
    if let Some(threads) = thread_pool_size {
        sandbox.thread_pool_size = threads;
    }
    if let Some(dur) = timeout {
        sandbox.timeout = Some(dur);
    }

    sandbox
}
