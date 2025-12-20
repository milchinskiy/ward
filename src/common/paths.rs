use std::path::PathBuf;

/// Returns Ward's data directory.
///
/// On Unix this follows XDG conventions:
/// - If `XDG_DATA_HOME` is set, `{XDG_DATA_HOME}/ward`
/// - Otherwise `{HOME}/.local/share/ward`
///
/// Falls back to the current directory if neither `XDG_DATA_HOME` nor `HOME` is set.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("ward");
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local").join("share").join("ward");
    }

    PathBuf::from(".").join("ward")
}
