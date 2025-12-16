use rust_args_parser as ap;

#[derive(Default, Debug)]
pub struct RunContext {
    file: std::path::PathBuf,
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
        .handler(|_, ctx: &mut super::Context| {
            let t = tokio::runtime::Runtime::new().map_err(ap::Error::user)?;
            t.block_on(ward::runner::run_file(
                ctx.run.file.as_path(),
                ward::runner::sandbox::SandboxPolicy::default(),
            ))
            .map_err(ap::Error::user)
        })
}
