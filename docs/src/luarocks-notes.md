Ward is designed to be self-contained and predictable.
For that reason, our compatibility target with LuaRocks is
intentionally conservative.

## What is supported

Ward works well with *LuaRocks* packages that are pure Lua
(i.e., they install only `*.lua` source files and do not require
any compiled native components).

To use pure-Lua rocks with Ward, install them into a LuaRocks tree
(local or global) and extend `package.path` so Ward can locate the modules.

Example (local tree):

```lua
-- Install with:
--   luarocks --lua-version 5.4 --tree ./.rocks install <rock>

local tree = "./.rocks"
package.path = table.concat({
  tree .. "/share/lua/5.4/?.lua",
  tree .. "/share/lua/5.4/?/init.lua",
  package.path,
}, ";")

local mod = require("some_rock_module")
```

Notes:

- Ward embeds Lua 5.4, so use `--lua-version 5.4`.
- Using a project-local tree (e.g., ./.rocks) is recommended for reproducibility.

## What is not supported (for now)

Ward does not currently provide reliable support for LuaRocks packages that include
native C extensions (e.g., `*.so`, `*.dll`) or depend on external Lua C modules.

You may be able to experiment by extending package.cpath, but compatibility is not
guaranteed and is not considered part of Ward's supported surface area.

### Why C-extension rocks are out of scope currently

Ward embeds Lua as part of the executable (via an embedded Lua runtime), which is
a deliberate choice to keep Ward:

- self-contained (no system Lua dependency),
- consistent across target environments,
- simple to install and run.

Most LuaRocks C modules are built under the assumption that they will load into
a process that links against a compatible, dynamically available Lua runtime,
and that their compiled binary can resolve the expected Lua symbols and ABI at
runtime. When the interpreter is embedded/self-contained, those assumptions may
not hold consistently across platforms, toolchains, and build modes.

### Roadmap stance

Supporting pure-Lua rocks provides most of the practical value for scripting
and library reuse while preserving Ward's simplicity. If and when Ward introduces
an optional build mode that links against a system-provided shared Lua runtime,
C-extension LuaRocks compatibility may become a supported configuration. Until
then, Ward's LuaRocks support should be considered **pure-Lua only**.
