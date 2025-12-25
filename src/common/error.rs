#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Lua(mlua::Error),
    Timeout(String),
    Exit(i32),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Lua(e) => e.fmt(f),
            Self::Timeout(e) => e.fmt(f),
            Self::Exit(e) => write!(f, "exit code: {e}"),
        }
    }
}

impl core::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<mlua::Error> for Error {
    fn from(e: mlua::Error) -> Self {
        Self::Lua(e)
    }
}

impl From<tokio::time::error::Elapsed> for Error {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        Self::Timeout("evaluation timed out".to_string())
    }
}
