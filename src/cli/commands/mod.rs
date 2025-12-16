pub mod run;

use rust_args_parser as ap;

#[derive(Default, Debug)]
pub struct Context {
    run: run::RunContext,
}

pub fn handle() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut ctx = Context::default();
    let env = ap::Env {
        color: ap::ColorMode::Auto,
        wrap_cols: 0,
        suggest: true,
        auto_help: true,
        version: Some(env!("CARGO_PKG_VERSION")),
        author: Some(env!("CARGO_PKG_AUTHORS")),
    };

    let root = ap::CmdSpec::new("root").help("Ward CLI").subcmd(run::command());

    match ap::parse(&env, &root, &args, &mut ctx) {
        Ok(_) => {},
        Err(ap::Error::ExitMsg { code, message }) => {
            if let Some(msg) = message {
                eprintln!("{msg}");
            }
            std::process::exit(code);
        },
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
