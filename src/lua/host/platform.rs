use mlua::{Lua, Table};
use sysinfo::System;

/// Initializes the `platform` module
/// # Errors [`mlua::Error`]
#[allow(clippy::too_many_lines)]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    // Target (compile-time)
    t.set("is_windows", lua.create_function(|_, ()| Ok(cfg!(target_os = "windows")))?)?;
    t.set("is_macos", lua.create_function(|_, ()| Ok(cfg!(target_os = "macos")))?)?;
    t.set("is_linux", lua.create_function(|_, ()| Ok(cfg!(target_os = "linux")))?)?;
    t.set("is_bsd", lua.create_function(|_, ()| Ok(is_bsd_target()))?)?;
    t.set("is_unix", lua.create_function(|_, ()| Ok(cfg!(unix)))?)?;

    t.set("os", lua.create_function(|_, ()| Ok(std::env::consts::OS.to_string()))?)?;
    t.set("arch", lua.create_function(|_, ()| Ok(std::env::consts::ARCH.to_string()))?)?;
    t.set(
        "platform",
        lua.create_function(|_, ()| Ok(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)))?,
    )?;

    // Runtime-ish (best-effort)
    t.set(
        "version",
        lua.create_function(|_, ()| Ok(System::long_os_version().unwrap_or_default()))?,
    )?;
    t.set(
        "release",
        lua.create_function(|_, ()| Ok(System::kernel_version().unwrap_or_default()))?,
    )?;
    t.set(
        "hostname",
        lua.create_function(|_, ()| Ok(System::host_name().unwrap_or_default()))?,
    )?;

    // Capabilities / conventions
    t.set(
        "exe_suffix",
        lua.create_function(|_, ()| Ok(if cfg!(windows) { ".exe" } else { "" }))?,
    )?;
    t.set(
        "path_sep",
        lua.create_function(|_, ()| Ok(if cfg!(windows) { "\\" } else { "/" }))?,
    )?;
    t.set(
        "env_sep",
        lua.create_function(|_, ()| Ok(if cfg!(windows) { ";" } else { ":" }))?,
    )?;
    t.set(
        "newline",
        lua.create_function(|_, ()| Ok(if cfg!(windows) { "\r\n" } else { "\n" }))?,
    )?;
    t.set(
        "endianness",
        lua.create_function(|_, ()| {
            Ok(if cfg!(target_endian = "little") {
                "little"
            } else {
                "big"
            })
        })?,
    )?;

    // Default shell invocation as { prog, args_table }
    t.set(
        "shell",
        lua.create_function(|lua, ()| {
            let arr = lua.create_table()?;
            if cfg!(windows) {
                arr.set(1, "cmd")?;
                arr.set(2, "/C")?;
            } else {
                arr.set(1, "sh")?;
                arr.set(2, "-lc")?;
            }
            Ok(arr)
        })?,
    )?;

    // One-shot info blob for scripts
    t.set(
        "info",
        lua.create_function(|lua, ()| {
            let info = lua.create_table()?;
            info.set("os", std::env::consts::OS)?;
            info.set("arch", std::env::consts::ARCH)?;
            info.set("platform", format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))?;
            info.set("is_windows", cfg!(windows))?;
            info.set("is_unix", cfg!(unix))?;
            info.set("is_bsd", is_bsd_target())?;
            info.set("version", System::long_os_version().unwrap_or_default())?;
            info.set("release", System::kernel_version().unwrap_or_default())?;
            info.set("hostname", System::host_name().unwrap_or_default())?;
            info.set(
                "endianness",
                if cfg!(target_endian = "little") {
                    "little"
                } else {
                    "big"
                },
            )?;
            info.set("exe_suffix", if cfg!(windows) { ".exe" } else { "" })?;
            info.set("path_sep", if cfg!(windows) { "\\" } else { "/" })?;
            info.set("env_sep", if cfg!(windows) { ";" } else { ":" })?;

            // shell = { "sh", "-lc" } / { "cmd", "/C" }
            let shell = lua.create_table()?;
            if cfg!(windows) {
                shell.set(1, "cmd")?;
                shell.set(2, "/C")?;
            } else {
                shell.set(1, "sh")?;
                shell.set(2, "-lc")?;
            }
            info.set("shell", shell)?;

            Ok(info)
        })?,
    )?;

    Ok(t)
}

const fn is_bsd_target() -> bool {
    cfg!(target_os = "freebsd")
        || cfg!(target_os = "netbsd")
        || cfg!(target_os = "openbsd")
        || cfg!(target_os = "dragonfly")
}
