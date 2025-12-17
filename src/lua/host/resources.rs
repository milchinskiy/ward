use mlua::{Lua, Table};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Initializes the `resources` module
/// # Errors [`mlua::Error`]
pub fn define(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    m.set(
        "get",
        lua.create_function(|lua, ()| {
            let mut sys = System::new_with_specifics(
                RefreshKind::nothing()
                    .with_memory(MemoryRefreshKind::nothing().with_ram())
                    .with_cpu(CpuRefreshKind::nothing().with_cpu_usage()),
            );
            sys.refresh_memory();
            sys.refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage()); // for cpu_count + load avg on some platforms

            let total = sys.total_memory();
            let available = sys.available_memory();
            let used = sys.used_memory();
            let free = sys.free_memory();

            let la = System::load_average();
            let logical = sys.cpus().len();
            let physical = System::physical_core_count().map(|n| n as u64);

            let t = lua.create_table()?;

            let mem = lua.create_table()?;
            mem.set("total", total)?;
            mem.set("available", available)?;
            mem.set("used", used)?;
            mem.set("free", free)?;
            t.set("memory", mem)?;

            let cpu = lua.create_table()?;
            let load = lua.create_table()?;
            load.set("1m", la.one)?;
            load.set("5m", la.five)?;
            load.set("15m", la.fifteen)?;
            cpu.set("load", load)?;

            let cores = lua.create_table()?;
            cores.set("logical", logical)?;
            cores.set("physical", physical)?;
            cpu.set("cores", cores)?;
            t.set("cpu", cpu)?;

            t.set("uptime", System::uptime())?;
            t.set("hostname", System::host_name().unwrap_or_default())?;

            Ok(t)
        })?,
    )?;

    Ok(m)
}
