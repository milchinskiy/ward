#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Lua(mlua::Error),
    Timeout(String),
    Tokio(tokio::time::error::Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Lua(e) => e.fmt(f),
            Self::Timeout(e) => e.fmt(f),
            Self::Tokio(e) => e.fmt(f),
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

impl From<tokio::time::error::Error> for Error {
    fn from(e: tokio::time::error::Error) -> Self {
        Self::Tokio(e)
    }
}
