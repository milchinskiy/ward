# Ward

Ward is a small, opinionated system-scripting runtime: **write scripts in Lua**,
run them with a single CLI, and get a standard library designed for "glue work"
(filesystem, processes, HTTP, templating, data formats, etc.).

The problem Ward is trying to solve is simple:
**shell scripting is powerful, but the "shell" is not one thing**.
Bash, sh, dash, busybox ash, zsh, and various platform quirks differ in ways
that are easy to forget and painful to debug. Quoting rules, error handling,
string processing, array semantics, and portability constraints turn
"one-liners" into fragile software.

Ward's approach is to keep the *good part* (compose existing tools) and replace
the *painful part* (shell dialect landmines) with:

- A real language (Lua 5.4) with predictable semantics
- A focused "system scripting" standard library under `ward.*`
- A single entry point (`ward run`) with sane argument forwarding and optional
sandboxing[*](#sandbox-limits) knobs

Detailed user guides live under `docs/` (mdBook). See `docs/src/SUMMARY.md` for the table of contents.

## What Ward is

- **CLI runtime** that runs a Lua file (Lua 5.4)
- **Built-in modules** exposed as `require("ward")` and `require("ward.<module>")`
- **Async-first execution model** so IO-bound operations can be expressed cleanly

## What Ward is not (current goals)

Ward is *not* trying to be a full orchestration framework. The near-term goal is
pragmatic:

- Replace ad-hoc Bash/sh/dash scripts with something **more reliable**
- Keep the surface area **small**, composable, and Lua-native
- Provide primitives that make daily tasks pleasant, without "inventing a new DSL"

Inventory/targeting, orchestration status renderers, and similar "automation
platform" features may exist later as separate modules or as part of `ward-lib`,
but they are not the current focus.

### Sandbox limits

It helps prevent runaway scripts (memory/time/instructions). This is not a
secure sandbox for untrusted code.

## Status

This project is early-stage. The current repository already includes
**integration tests covering the core modules** and the CLI runner. Expect breaking
changes while APIs settle.

## Quick start

### Binary (github releases)

```bash
curl -fsSL https://raw.githubusercontent.com/milchinskiy/ward/master/install.sh | bash
```

or

```bash
curl -fsSL https://raw.githubusercontent.com/milchinskiy/ward/master/install.sh \
    | WARD_FLAVOR=musl WARD_VERSION=v0.1.2 INSTALL_DIR=~/.local/bin \
    bash
```

### Build

```bash
cargo build --release
```

Binary will be at:

```bash
./target/release/ward
```

### Run a script

```bash
ward run path/to/script.lua -- arg1 arg2
# or
ward run path/to/script.lua arg1 arg2
```

Notes:

- Everything after `FILE` is forwarded to the script and exposed as the standard
Lua global `arg`.
- Use `--` to terminate Ward's CLI parsing explicitly (recommended when you
forward arbitrary flags).

### Shebang mode

Ward strips a leading shebang (`#!`) line from Lua files, so you can make
scripts executable.

On systems with `env -S` support:

```lua
#!/usr/bin/env -S ward run
print("hello from ward")
```

Then:

```bash
chmod +x ./script.lua
./script.lua
```

## CLI reference

Currently the main command is:

```sh
ward run [OPTIONS] FILE [--] [ARGS...]
```

Options:

- `-m, --memory-limit <bytes>`: memory cap for the Lua VM (bytes)
- `-i, --instruction-limit <n>`: approximate instruction cap (enforced via VM hook)
- `-t, --threads <n>`: worker thread count
- `-T, --timeout <duration>`: timeout (`500ms`, `2s`, `1m`, or plain seconds)

Logging:

- `WARD_LOG=trace|debug|info|warn|error|fatal` controls runtime logging level.

## Your first scripts

### 1) Filesystem + simple output

```lua
local fs = require("ward.fs")

local path = "hello.txt"
fs.write(path, "hello from ward\n")
print("wrote", path, "exists?", fs.is_exists(path))
```

### 2) Run a command and capture output

```lua
local process = require("ward.process")
local res = process.cmd("uname", "-a"):output()

if not res.ok then
  error("uname failed with code " .. tostring(res.code))
end

print(res.stdout)
```

### 3) Pipes, the safe kind

Ward exposes a pipeline operator for commands (Lua 5.4 bitwise OR `|`):

```lua
local p = require("ward.process")

local pipeline =
  p.cmd("printf", "a\nb\nc\n")
  | p.cmd("grep", "b")

local out = pipeline:output()
print(out.stdout)
```

### 4) HTTP + JSON

```lua
local http = require("ward.net.http")
local json = require("ward.convert.json")

local r = http.get("https://example.com/api", {
  headers = { ["accept"] = "application/json" },
})

if not r:is_ok() then
  error("request failed: " .. tostring(r:status()))
end

local body = r:body() or "{}"
local obj = json.decode(body)
print("keys:", obj and "decoded" or "nil")
```

### 5) Templating (MiniJinja)

```lua
local mj = require("ward.template.minijinja")

local rendered = mj.render("Hello {{ name }}!", { name = "Ward" })
print(rendered)
```

## Built-in modules (high level)

Ward exposes modules under `ward.*` (and also registers each as a standalone
require target):

- `ward.async` - tasks/channels/await helpers for concurrent scripting
- `ward.convert.{json,toml,yaml,ini}` - parse/emit common config formats
- `ward.crypto` - hashes (sha256/sha1/md5) for bytes and files
- `ward.env` - environment helpers (including controlled overlays for subprocesses)
- `ward.fs` - filesystem ops and path helpers
- `ward.helpers.number` - number helpers
- `ward.helpers.retry` - retry helpers
- `ward.helpers.string` - string helpers
- `ward.helpers.table` - table helpers
- `ward.host.platform` - platform helpers
- `ward.host.resources` - host resource helpers
- `ward.io` - stdin/stdout/stderr/flush helpers
- `ward.ipc.unix` - fast local IPC using Unix domain sockets (client/server)
- `ward.lifecycle` - graceful shutdown semantics (to ensure cleanup)
- `ward.log` - logging helpers
- `ward.module` - manage external Lua modules under the Ward data directory
- `ward.net.fetch` - download URLs or Git repos into a directory
- `ward.net.http` - HTTP client (GET/POST/PUT/PATCH/DELETE/...)
- `ward.process` - structured process execution, pipelines, middlewares, exit handling
- `ward.template.minijinja` - string/file rendering via MiniJinja
- `ward.term` - terminal/console helpers
- `ward.time` - time and duration helpers

## ward-lib

Ward itself aims to stay small and focused on primitives.

**`ward-lib`** is a separate "standard library" repository under active
development, intended to provide:

- Practical wrappers around common CLI tools (git, curl/wget, systemd, tar, jq, etc.)
- Reusable snippets for daily automation tasks
- Higher-level "batteries" while keeping Ward core lean

The intent is: **Ward stays stable and minimal**, while `ward-lib` can evolve
quickly as a curated toolbox.

## External modules via `ward.module`

Ward installs a dedicated `require("<name>")` searcher. External
modules are stored under Ward's data directory (XDG-style on Unix), for example:

- `${XDG_DATA_HOME}/ward/externals/.store/<id>` or `~/.local/share/ward/externals/.store/<id>`

Checkouts live in a content-addressed store where `<id> = sha256(normalized URL +
selector)`. For Git, the selector is one of `rev:<sha>`, `tag:<name>`,
`branch:<name>`, or `head` (default). `ward.module.git/url` binds the logical
name for the current Ward run, so multiple revisions can coexist on disk while
scripts keep the ergonomic `require("foo")`.

Rebinding the same name to a different revision within a single run is rejected
unless `force=true`, which also clears any cached module so the next `require`
reloads it.

Example (clone a Git repo into externals, then require it):

```lua
local m = require("ward.module")

local info = m.git("https://github.com/<org>/<repo>.git", {
  -- name = "my-lib",  -- optional override
  -- rev = "<sha>",    -- optional
  -- branch = "main",  -- optional
  -- force = true,     -- optional
})

print("installed at:", info.store_path)
print("store id:", info.id)
-- Then: local lib = require(info.require)
-- or
-- local lib = require("my_lib")
```

## Development

### Tests

```bash
cargo test
```

### Nix

A `flake.nix` is included for a development shell:

```bash
nix develop
```

### Style / lint

```bash
cargo fmt
cargo clippy
```

## Vision (in one paragraph)

Ward is a deliberate attempt to make "small systems code" boring again:
fewer dialect traps than shell, fewer dependencies than Python/Node for
simple tasks, and a standard library that treats processes, files, and
IO as first-class primitives. The end-state is a scripting toolchain where
you can confidently write and share scripts without asking "which shell
does this run under?".

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
