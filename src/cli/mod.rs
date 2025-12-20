pub mod commands;

pub fn init_logger() {
    let level = match std::env::var("WARD_LOG").ok().as_deref() {
        Some("trace") => rustlog::Level::Trace,
        Some("debug") => rustlog::Level::Debug,
        Some("warn" | "warning") => rustlog::Level::Warn,
        Some("error") => rustlog::Level::Error,
        Some("fatal") => rustlog::Level::Fatal,
        _ => rustlog::Level::Info,
    };
    rustlog::set_level(level);
    rustlog::set_show_file_line(true);
    rustlog::set_show_thread_id(false);
}
