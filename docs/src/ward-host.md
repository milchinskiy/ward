- [`host.platform`](#hostplatform)
- [`host.resources`](#hostresources)

```lua
local host = require("ward.host")
local platform = require("ward.host.platform")
local resources = require("ward.host.resources")
```

### `host.platform`

Platform inspection helpers (OS, arch, etc.).

- `platform.is_windows() -> boolean`
- `platform.is_macos() -> boolean`
- `platform.is_linux() -> boolean`
- `platform.is_bsd() -> boolean`
- `platform.is_unix() -> boolean`
- `platform.os() -> string` (e.g., `"linux"`)
- `platform.arch() -> string` (e.g., `"x86_64"`)
- `platform.platform() -> string` (`"<os>-<arch>"`)
- `platform.version() -> string` (long OS version, best-effort)
- `platform.release() -> string` (kernel version, best-effort)
- `platform.hostname() -> string`
- `platform.exe_suffix() -> string` (`".exe"` on Windows, else `""`)
- `platform.path_sep() -> string` (`"/"` or `"\\"`)
- `platform.env_sep() -> string` (`":"` or `";"`)
- `platform.newline() -> string` (`"\r\n"` on Windows, else `"\n"`)
- `platform.endianness() -> string` (`"little"` or `"big"`)
- `platform.shell() -> table` - array-like command for the default shell:
  - Unix: `{ "sh", "-lc" }`
  - Windows: `{ "cmd", "/C" }`
- `platform.info() -> table` - one-shot snapshot containing the fields above
  plus `is_windows`, `is_unix`, `is_bsd`, `platform`, `shell`.

### `host.resources`

Resource inspection (memory, CPU) for the host process.

- `resources.get() -> table` returns:
  - `memory.total`, `memory.available`, `memory.used`, `memory.free` (bytes)
  - `cpu.load["1m"]`, `cpu.load["5m"]`, `cpu.load["15m"]` (load averages)
  - `cpu.cores.logical` (integer), `cpu.cores.physical` (integer|nil)
  - `uptime` (seconds)
  - `hostname` (string, best-effort)
