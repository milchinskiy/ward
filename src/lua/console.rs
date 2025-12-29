use std::sync::Arc;
use tokio::io;
use tokio::sync::Mutex;

pub struct Console {
    pub stdin: Mutex<tokio::io::BufReader<io::Stdin>>,
    pub stdout: Mutex<io::Stdout>,
    pub stderr: Mutex<io::Stderr>,
    pub interactive: Mutex<()>,
}

impl Console {
    #[must_use]
    pub fn new() -> Self {
        Self {
            stdin: Mutex::new(tokio::io::BufReader::new(io::stdin())),
            stdout: Mutex::new(io::stdout()),
            stderr: Mutex::new(io::stderr()),
            interactive: Mutex::new(()),
        }
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn console(lua: &mlua::Lua) -> Arc<Console> {
    if let Some(c) = lua.app_data_ref::<Arc<Console>>() {
        return c.clone();
    }
    let c = Arc::new(Console::new());
    lua.set_app_data(c.clone());
    c
}
