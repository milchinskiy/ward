pub mod commands;

pub fn init_logger() {
    rustlog::set_level(rustlog::Level::Trace);
    rustlog::set_show_file_line(true);
    rustlog::set_show_thread_id(false);
}
