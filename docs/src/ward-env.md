# ward.env

- [`env.get(key, default?) -> string|nil`](#envgetkey-default---stringnil)
- [`env.set(key, value) -> boolean`](#envsetkey-value---boolean)
- [`env.export(key, value?) -> boolean`](#envexportkey-value---boolean)
- [`env.unset(key) -> boolean`](#envunsetkey---boolean)
- [`env.clear() -> nil`](#envclear---nil)
- [`env.list() -> table`](#envlist---table)
- [`env.is_exists(key) -> boolean`](#envis_existskey---boolean)
- [`env.hostname() -> string`](#envhostname---string)
- [`env.which(name) -> string|nil`](#envwhichname---stringnil)
- [`env.is_in_path(path_or_name) -> boolean`](#envis_in_pathpath_or_name---boolean)

Ward uses an environment overlay:

- `env.set` / `env.unset` / `env.clear` modify Ward's overlay only (they do
not mutate the process-global OS environment).
- Read operations (`env.get` / `env.list` / `env.is_exists` / `env.which` /
`env.is_in_path`) resolve the effective environment: the process environment
plus overlay modifications (overlay wins).
- The overlay is applied to child processes spawned via `ward.process` and to
the git invocations used by `ward.net.fetch.git`.
For child processes, precedence is: process env → Ward overlay → per-command
overrides (Cmd:env / Cmd:envs).

Used to inspect and modify environment/Ward variables.
Mutations are applied to the current process via local Ward env variables
overlay to keep it safe in async contexts.

```lua
local env = require("ward.env")
```

### `env.get(key, default?) -> string|nil`

Get environment variable `key`. If missing (or `key` is empty), returns `default`.

```lua
local home = env.get("HOME")
local port = env.get("PORT", "8080")
```

### `env.set(key, value) -> boolean`

Set an environment variable in the Ward overlay. Returns `false` if the key
is invalid (empty, contains `=`, or contains `\0`) or the value contains `\0`.

```lua
env.set("FOO", "bar")
```

### `env.export(key, value?) -> boolean`

Mutate the *process* environment (not just Ward's overlay). This mirrors
`export` in shells and affects concurrently running scripts in the same
process, so use it sparingly.

- `value` omitted / `nil` ⇒ removes the variable from the process environment
and overlay.
- Returns `false` on invalid keys.

```lua
-- Prefer env.set for isolation; use export only when you must change 
-- the process env.
env.export("PATH", "/custom/bin:" .. (env.get("PATH") or ""))
```

### `env.unset(key) -> boolean`

Remove an environment variable (from the overlay). Returns `false` if the key
is invalid.

```lua
env.unset("FOO")
```

### `env.clear() -> nil`

Clears all overlay modifications (restores the effective environment back to
the base process environment).

### `env.list() -> table`

Returns a table of the effective environment (base process env with overlay applied).

Example (print all):

```lua
local t = env.list()
for k, v in pairs(t) do
  print(k, v)
end
```

### `env.is_exists(key) -> boolean`

Returns `true` if variable exists in the effective environment.

```lua
if env.is_exists("CI") then
  print("running in CI")
end
```

### `env.hostname() -> string`

Returns hostname.

### `env.which(name) -> string|nil`

Searches `PATH` (and Windows `PATHEXT`) for an executable.

```lua
local git = env.which("git")
assert(git, "git not found")
```

### `env.is_in_path(path_or_name) -> boolean`

Returns whether a candidate is reachable via `PATH` search.
