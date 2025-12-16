mod cli;

fn main() {
    cli::init_logger();
    cli::commands::handle();
}
