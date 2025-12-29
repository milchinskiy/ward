# Ward API user guide

This document is a user-facing reference for the Ward Lua runtime: modules, functions, userdata types, and common patterns.

---

## 1. Mental model

Ward is a Lua runtime designed for writing scripts that feel like bash/sh, but with:

- Explicit modules (via `require("ward.*")`)
- Async-capable primitives (I/O, processes, timers)
- Stronger typing at the API boundary (option tables, structured results)

### 1.1 Importing

You can import either the root module or submodules:

```lua
local ward   = require("ward")
local fs     = require("ward.fs")
local path   = require("ward.fs.path")
local io     = require("ward.io")
local proc   = require("ward.process")
local crypto = require("ward.crypto")
local retry  = require("ward.helpers.retry")
local time   = require("ward.time")
local term   = require("ward.term")
```

### 1.2 Async in Ward: two styles

Ward exposes async behavior in two ways:

1) **Async functions** (implemented with `create_async_function`): you call them normally and get the value back.

2) **Awaitable userdata**: you receive an object that must be explicitly awaited.

Ward uses awaitable userdata for interactive input and for some timer constructs.

#### Awaiting an awaitable

If a function returns an awaitable userdata, you must either:

- Call `:wait()`
- Or call the object itself (it implements `__call`)

```lua
local a = term.confirm({ question = "Continue?", default = false })
local ok = a:wait()   -- explicit
-- or
local ok = a()        -- idiomatic, shorter
```

A common pitfall:

```lua
local ok = term.confirm({ question = "Continue?" })
if ok then
  -- WRONG: userdata is always truthy
end
```

---

## 2. Conventions used in this guide

- `nil|string` means the function returns either `nil` or a Lua string.
- "bytes string" means a Lua string whose contents are raw bytes (binary-safe).
- Paths are typically accepted as strings (and in most ward.fs APIs also as ward.fs.path objects).
- Most option tables are optional; when omitted, defaults apply.

---

### 2.1 CLI sandbox limits (`ward run`)

`ward run` executes a Lua file under a configurable sandbox. The most commonly used switches are:

- `--memory-limit BYTES` — maximum Lua memory usage
- `--instruction-limit N` — approximate instruction budget (see notes below)
- `--timeout SECONDS` — wall-clock timeout (also accepts duration strings like `500ms`, `2s`, `1m`)
- `--threads N` — Tokio worker threads

Notes:

- **Instruction limiting is intentionally coarse.** Ward installs a Lua VM hook every 1024 instructions (or less if the configured limit is smaller), so a script may exceed the configured limit by up to 1023 instructions.
- The instruction hook does not execute while awaiting Rust async operations (e.g., I/O). Long-running I/O does not consume the instruction budget.

## 3. `ward.env` — environment and PATH tools

Ward uses an environment overlay:

- `env.set` / `env.unset` / `env.clear` modify Ward's overlay only (they do not mutate the process-global OS environment).
- Read operations (`env.get` / `env.list` / `env.is_exists` / `env.which` / `env.is_in_path`) resolve the effective environment: the process environment plus overlay modifications (overlay wins).
- The overlay is applied to child processes spawned via `ward.process` and to the git invocations used by `ward.net.fetch.git`.
For child processes, precedence is: process env → Ward overlay → per-command overrides (Cmd:env / Cmd:envs).

Used to inspect and modify environment/Ward variables.
Mutations are applied to the current process via local Ward env variables overlay to
keep it safe in async contexts.

```lua
local env = require("ward.env")
```

### 3.1 `env.get(key, default?) -> string|nil`

Get environment variable `key`. If missing (or `key` is empty), returns `default`.

```lua
local home = env.get("HOME")
local port = env.get("PORT", "8080")
```

### 3.2 `env.set(key, value) -> boolean`

Set an environment variable in the Ward overlay. Returns `false` if the key is invalid (empty, contains `=`, or contains `\0`) or the value contains `\0`.

```lua
env.set("FOO", "bar")
```

### 3.3 `env.export(key, value?) -> boolean`

Mutate the *process* environment (not just Ward's overlay). This mirrors `export` in shells and affects concurrently running scripts in the same process, so use it sparingly.

- `value` omitted / `nil` ⇒ removes the variable from the process environment and overlay.
- Returns `false` on invalid keys.

```lua
-- Prefer env.set for isolation; use export only when you must change the process env.
env.export("PATH", "/custom/bin:" .. (env.get("PATH") or ""))
```

### 3.4 `env.unset(key) -> boolean`

Remove an environment variable (from the overlay). Returns `false` if the key is invalid.

```lua
env.unset("FOO")
```

### 3.5 `env.clear() -> nil`

Clears all overlay modifications (restores the effective environment back to the base process environment).

### 3.6 `env.list() -> table`

Returns a table of the effective environment (base process env with overlay applied).

Example (print all):

```lua
local t = env.list()
for k, v in pairs(t) do
  print(k, v)
end
```

### 3.7 `env.is_exists(key) -> boolean`

Returns `true` if variable exists in the effective environment.

```lua
if env.is_exists("CI") then
  print("running in CI")
end
```

### 3.8 `env.hostname() -> string`

Returns hostname.

### 3.9 `env.which(name) -> string|nil`

Searches `PATH` (and Windows `PATHEXT`) for an executable.

```lua
local git = env.which("git")
assert(git, "git not found")
```

### 3.10 `env.is_in_path(path_or_name) -> boolean`

Returns whether a candidate is reachable via `PATH` search.

---

## 4. `ward.fs` — filesystem, paths, globbing, temporary dirs

```lua
local fs = require("ward.fs")
```

### 4.1 Existence and type checks

- `fs.is_exists(path) -> boolean`
- `fs.is_dir(path) -> boolean`
- `fs.is_file(path) -> boolean`
- `fs.is_link(path) -> boolean`
- `fs.is_symlink(path) -> boolean`
- `fs.is_block_device(path) -> boolean` (Unix)
- `fs.is_char_device(path) -> boolean` (Unix)
- `fs.is_fifo(path) -> boolean` (Unix)
- `fs.is_socket(path) -> boolean` (Unix)
- `fs.is_executable(path) -> boolean`
- `fs.is_readable(path) -> boolean`
- `fs.is_writable(path) -> boolean`

Notes:

- For files, `fs.is_readable/fs.is_writable` check whether the file can be opened for read/write.
- For directories, `fs.is_readable` checks whether the directory can be listed (`read_dir`), and
  `fs.is_writable` checks whether a temporary file can be created and removed inside the directory.

Example:

```lua
if fs.is_file("./build.sh") and fs.is_executable("./build.sh") then
  print("runnable")
end
```

### 4.2 Path utilities

- `fs.readlink(path) -> string|nil`
- `fs.realpath(path) -> string|nil`
- `fs.dirname(path) -> string`
- `fs.basename(path) -> string`
- `fs.join(a, b, ...) -> string`

Example:

```lua
local p = fs.join("build", "out", "app.bin")
print(fs.dirname(p))
print(fs.basename(p))
```

#### 4.2.1 `fs.path` — pure path manipulation (Path userdata)

`fs.path` provides a `Path` userdata type for manipulating paths **without** touching the filesystem.
It is useful for building paths safely and passing them to `ward.fs` APIs.

Constructors:

- `fs.path.new(path) -> Path`
- `fs.path.cwd() -> Path`
- `fs.path.join(a, b) -> Path` (both arguments may be strings or `Path`)

Methods on `Path`:

- `Path:is_abs() -> boolean`
- `Path:normalize() -> Path`
- `Path:parts() -> table` (array-like, path components)
- `Path:split() -> (dirname: string, basename: string)`
- `Path:join(segment) -> Path`
- `Path:dirname() -> string`
- `Path:basename() -> string`
- `Path:extname() -> nil|string`
- `Path:stem() -> nil|string`
- `Path:as_string() -> string`

Interoperability:

Most `ward.fs` functions accept either a path string **or** a `fs.path` object.

Example:

```lua
local fs = require("ward.fs")
local path = require("ward.fs.path")

local p = path.new("build/../out/app.bin"):normalize()
assert(fs.mkdir(p:dirname(), { recursive = true, force = true }).ok)
assert(fs.write(p, "hello\n").ok)
print("wrote:", tostring(p))
```

### 4.3 Directory listing and globbing

#### `fs.list(path, opts?) -> table`

Returns an array-like table of paths.

Options (`opts` table):

- `recursive` (boolean, default `false`)
- `depth` (integer, default `0`) — recursion depth limit; `0` means unlimited
- `dirs` (boolean, default `false`) — include directories
- `files` (boolean, default `false`) — include files
- `regex` (string|nil) — regex filter applied to the full path string

Notes:

- If both `dirs` and `files` are `false` (the default), both directories and files are included.
- Ordering is OS-dependent (Ward does not currently sort results).

Examples:

```lua
-- list everything
for _, p in ipairs(fs.list(".")) do
  print(p)
end

-- only files, recursive, depth-limited
local files = fs.list("src", { recursive = true, depth = 3, files = true })
```

#### `fs.glob(pattern) -> table`

Returns an array-like table of paths matching a glob pattern.

```lua
for _, p in ipairs(fs.glob("src/**/*.rs")) do
  print(p)
end
```

### 4.4 Directories and removal

**Result convention for mutating operations:** most functions that change the
filesystem return a table `{ ok, err }` where `ok` is a boolean and `err` is
a string (or `nil` on success). When `ok` is `false`, `err` is intended to be
human-readable and suitable for printing/logging.

#### `fs.mkdir(path, opts?) -> { ok, err }`

Create directory.

Options:

- `recursive` (boolean, default `false`)
- `mode` (number, unix-only; default `0o755`)
- `force` (boolean, default `false`) — treat “already exists” as success

```lua
assert(fs.mkdir("build", { recursive = true, mode = 0o755 }).ok)
```

#### `fs.rm(path, opts?) -> { ok, err }`

Remove file or directory.

Options:

- `recursive` (boolean, default `false`) — required for directories
- `force` (boolean, default `false`) — treat missing-path as success

```lua
assert(fs.rm("build", { recursive = true, force = true }).ok)
```

#### `fs.unlink(path, opts?) -> { ok, err }`

Remove a file (like `rm -f file`).

### 4.5 Permissions and links

- `fs.chmod(path, mode) -> { ok, err }`
- `fs.chown(path, uid, gid) -> { ok, err }` (Unix)
- `fs.rename(from, to) -> { ok, err }`
- `fs.link(from, to) -> { ok, err }` (hard link)
- `fs.symlink(from, to) -> { ok, err }` (symbolic link)

### 4.6 Timestamps

#### `fs.touch(path, opts?) -> { ok, err }`

Create file if missing and update timestamps.

Options:

- `recursive` (boolean, default `false`) — create parent directories first

```lua
assert(fs.touch("logs/app.log", { recursive = true }).ok)
```

### 4.7 File IO

#### `fs.read(path, opts?) -> bytes string`

Reads a file and returns a Lua string containing raw bytes.

Options:

- `mode` (`"text"|"binary"`, default `"text"`)

Notes:

- In `"text"` mode Ward validates UTF-8; the returned Lua string still contains the original bytes.

```lua
local data = fs.read("README.md")
```

#### `fs.write(path, data, opts?) -> { ok, err }`

Write data to a file.

Options (selected):

- `mode` (`"overwrite"|"append"|"prepend"|"binary"`, default `"overwrite"`)
- `append` (boolean, optional convenience; equivalent to `mode="append"`)
- `binary` (boolean, default `false`) — convert `data` as bytes

Notes:

- Ward does not automatically create parent directories; combine with `fs.mkdir(fs.dirname(path), {recursive=true})`.

```lua
assert(fs.write("out.txt", "hello\n").ok)
assert(fs.write("out.txt", "more\n", { mode = "append" }).ok)
```

### 4.8 Copy and move

- `fs.copy(from, to, opts?) -> { ok, err }`
- `fs.move(from, to, opts?) -> { ok, err }`

Notes:

- `fs.copy` operates on regular files; it does not copy directories.
- `fs.move` uses `rename` when possible. On cross-device moves it falls back to copy+remove for regular files; moving directories or symlinks across devices is currently unsupported.

### 4.9 Temporary directories

#### `fs.tempdir(prefix?) -> string`

Creates a temporary directory and returns its path.

```lua
local dir = fs.tempdir("ward-")
print("tmp:", dir)
```

---

## 5. `ward.io` — stdin/stdout/stderr (async)

```lua
local io = require("ward.io")
```

Ward serializes reads/writes with internal mutexes so concurrent operations do not interleave unpredictably.

### 5.1 `io.read_all(opts?) -> bytes string`

Reads all remaining stdin into a string.

Optional `opts`:

- `max_bytes` (number|integer) — if provided, fails when stdin exceeds this limit.

```lua
local s = io.read_all()

-- hard cap (1 MiB)
local s2 = io.read_all({ max_bytes = 1024 * 1024 })
```

### 5.2 `io.read_line() -> byts string|nil`

Reads one line from stdin.

- Returns `nil` on EOF.

```lua
local line = io.read_line()
if line == nil then return end
print("got:", line)
```

### 5.3 `io.read_lines() -> function`

Returns an iterator-like function. Each call reads one line from stdin and returns `bytes string|nil` (nil on EOF).

```lua
local next_line = io.read_lines()
while true do
  local line = next_line()
  if line == nil then break end
  print(line)
end
```

### 5.4 Output

- `io.write_stdout(data) -> true`
- `io.write_stderr(data) -> true`
- `io.flush_stdout() -> nil`
- `io.flush_stderr() -> nil`

```lua
io.write_stdout("hello")
io.write_stderr("warn\n")
io.flush_stdout()
```

---

## 6. `ward.process` — run external programs and pipelines

```lua
local process = require("ward.process")
```

### 6.1 Constructors

#### `process.cmd(program, ...args) -> Cmd`

Create a command.

Arguments may be provided either as varargs or as a single array-table:

```lua
local a = process.cmd("git", "status", "--porcelain")
local b = process.cmd("git", { "status", "--porcelain" })
```

#### `process.sh(script) -> Cmd`

Run a shell fragment using the platform default shell:

- Unix: `sh -lc <script>`
- Windows: `cmd /C <script>`

```lua
local cmd = process.sh("echo $HOME")
```

#### `process.exit(code?) -> (raises)`

Request script termination with an exit status.

- This does **not** terminate the host process immediately.
- Ward requests shutdown, unwinds execution, runs shutdown handlers, and then the CLI exits with the given status.
- `code` defaults to `0`. Negative values are coerced to `1`. Values above `i32::MAX` are clamped.

Use this for early returns from scripts that still need cleanup handlers to run.

#### `process.shell_defaults(opts?) -> table`

Set or read defaults that mimic common `set -euo pipefail` toggles:

- `pipefail` (boolean) — when true, new commands/pipelines treat any non-zero step as failure by default.
- `timeout` (number|string|nil) — pipeline-level timeout for new commands/pipelines. Accepts integers (ms) or human strings (`500ms`, `2s`, `1m`). `nil` clears the default.

Returns a table with the active defaults. Example:

```lua
local process = require("ward.process")

-- Enable pipefail + a 30s default timeout for all new cmds/pipelines.
process.shell_defaults({ pipefail = true, timeout = "30s" })
```

### 6.2 `CmdResult` userdata (result of run/output)

Fields:

- `result.ok` (boolean)
- `result.code` (integer)
- `result.signal` (integer|nil)
- `result.stdout` (bytes string|nil)
- `result.stderr` (bytes string|nil)
- `result.steps` (table of integers) — per-step exit codes

Methods:

- `result:is_ok() -> boolean`
- `result:assert_ok(msg?) -> ()` — throws a Lua error if not ok

Example:

```lua
local r = process.cmd("echo", "hi"):output()
print(r.ok, r.code)
print(r.stdout)
r:assert_ok("echo failed")
```

### 6.3 `Cmd` userdata

In addition to one-shot execution (`run` / `output`), `Cmd` supports spawning long-running processes and streaming their stdio.

Builder methods (fluent):

- `cmd:cwd(path) -> Cmd`
- `cmd:env(key, value) -> Cmd`
- `cmd:envs(tbl) -> Cmd` (reads key/value pairs)
- `cmd:timeout(ms|duration_string|nil) -> Cmd`
- `cmd:stdin(data) -> Cmd` (bytes string)
- `cmd:stdin_file(path) -> Cmd`
- `cmd:stdin_null() -> Cmd`
- `cmd:stderr_to_stdout(true|false) -> Cmd`
- `cmd:spawn(opts?) -> ProcChild`

Notes:

- `cmd:stdin(data)`, `cmd:stdin_file(path)`, and `cmd:stdin_null()` configure stdin for `run()` / `output()` / `spawn()`. This is **one-shot** input: Ward will feed the configured input (or connect stdin to the file/null stream) and then close stdin.
- `cmd:stdin(v)` accepts only a bytes string, or `nil`/`false` to reset to inherited stdin. Other values raise an error.
- `cmd:stdin_file(path)` fails at spawn-time if the file cannot be opened.
- `cmd:stderr_to_stdout(true)` merges stderr into stdout **best-effort**. Ordering may differ from shell `2>&1`. In capture mode (`output()`), merged data is returned in `stdout` and `stderr` is `nil`.

- `cmd:timeout(...)` accepts milliseconds (number) or human strings like `"500ms"`, `"2s"`, `"1m"`. Pass `nil` to clear.

- `cmd:stdin_null()` sets stdin to a closed stream (equivalent to shell `< /dev/null`).
- `cmd:pipe(other_cmd_or_pipeline) -> Pipeline`

Terminal operations:

- `cmd:run() -> CmdResult` — inherits stdio
- `cmd:output() -> CmdResult` — captures stdout/stderr

Pipeline operator:

- `cmd1 | cmd2` produces a `Pipeline` (via `__bor`).

Example:

```lua
local r = process.cmd("git", "rev-parse", "HEAD"):output()
r:assert_ok()
print(r.stdout)
```

#### 6.3.1 `Cmd:spawn(opts?) -> ProcChild`

Spawns a long-running child process and returns a `ProcChild` handle. This is used for:

- Streaming stdout/stderr incrementally (lines or raw bytes)
- Interactive processes (writing to stdin)
- Long-running daemons/subscriptions (e.g., `pw-mon`, `tail -f`, etc.)

`opts` is an optional table:

- `stdin`  (boolean or `"pipe"|"inherit"|"null"`)
  - `true` / `"pipe"`: pipe stdin so Lua can write via `ProcChild:stdin()`
  - `false` / `"inherit"`: inherit parent stdin
  - `"null"`: connect stdin to a closed stream
  - Default: inferred. If you set `cmd:stdin(data)`, Ward pipes stdin to feed the bytes; if you set `cmd:stdin_null()`, stdin is null; if you set `cmd:stdin_file(path)`, Ward pre-opens the file and uses it as stdin. Otherwise stdin defaults to inherited stdin unless you request piping.

- `stdout` (boolean or `"pipe"|"inherit"|"null"`, default `true`)
  - `true` / `"pipe"`: pipe stdout so you can stream it
  - `false` / `"inherit"`: inherit parent stdout
  - `"null"`: discard stdout

- `stderr` (boolean or `"pipe"|"inherit"|"null"`)
  - `true` / `"pipe"`: pipe stderr so you can stream it
  - `false` / `"inherit"`: inherit parent stderr
  - `"null"`: discard stderr
  - Default: `true` when `cmd:stderr_to_stdout(true)` is set, otherwise `false` (`"inherit"`).

Important:

- If you call `cmd:stderr_to_stdout(true)`, Ward merges stderr into the stdout stream (similar to `2>&1`). In this case, stderr is not available separately and must be read from stdout. Ordering is best-effort and may not exactly match OS-level `2>&1` interleaving.
- Choose either one-shot stdin via `cmd:stdin(...)` / `cmd:stdin_file(...)` / `cmd:stdin_null()`, or interactive stdin via `spawn({ stdin = true })` + `ProcChild:stdin()`. Combining `cmd:stdin(...)` / `cmd:stdin_file(...)` with `spawn({ stdin = true })` is invalid and raises an error.

Example (spawn + line streaming):

```lua
local p = require("ward.process")

local child = p.cmd("sh", "-lc", "printf 'a\\nb\\n' && sleep 1"):spawn({ stdout = true })
local out = assert(child:stdout_lines())

while true do
  local line, err = out:wait()
  if not line then break end
  print("line:", line)
end

child:wait()
```

#### 6.3.2 `ProcChild` userdata

Represents a spawned child process.

Methods:

- `child:pid() -> integer`
- `child:pids() -> table` — array of PIDs for all stages (for pipelines)
- `child:stdin() -> ProcStdin | nil, err`
  - Returns `nil, "not_piped"` if stdin is not piped.
- `child:stdout_lines() -> LineStream | nil, err`
  - Returns `nil, "not_piped"` if stdout is not piped.
  - Returns `nil, "mode_conflict"` if stdout was already opened as bytes.
  - You may call this multiple times; each handle reads from the same underlying pipe.
- `child:stderr_lines() -> LineStream | nil, err`
  - Returns `nil, "not_piped"` if stderr is not piped.
  - Returns `nil, "merged"` if stderr was merged into stdout via `cmd:stderr_to_stdout(true)`.
  - Returns `nil, "mode_conflict"` if stderr was already opened as bytes.
  - You may call this multiple times; each handle reads from the same underlying pipe.
- `child:stdout_bytes() -> ByteStream | nil, err`
  - Returns `nil, "not_piped"` if stdout is not piped.
  - Returns `nil, "mode_conflict"` if stdout was already opened as lines.
  - You may call this multiple times; each handle reads from the same underlying pipe.
- `child:stderr_bytes() -> ByteStream | nil, err`
  - Returns `nil, "not_piped"` if stderr is not piped.
  - Returns `nil, "merged"` if stderr was merged into stdout via `cmd:stderr_to_stdout(true)`.
  - Returns `nil, "mode_conflict"` if stderr was already opened as lines.
  - You may call this multiple times; each handle reads from the same underlying pipe.
- `child:kill() -> boolean` (async)
- `child:wait() -> CmdResult` (async)

`ProcChild:wait()` returns a `CmdResult` with:

- `ok`, `code`, `signal`
- `stdout`/`stderr` are typically `nil` because streaming consumption is incremental, not captured.

#### 6.3.3 `ProcStdin` userdata (interactive stdin)

Returned by `ProcChild:stdin()` when stdin is piped.

Methods:

- `stdin:write(bytes_string) -> true | nil, err` (async)
- `stdin:writeln(string) -> true | nil, err` (async) — writes string + `\\n`
- `stdin:flush() -> true | nil, err` (async)
- `stdin:close() -> true` (async)
- `stdin:is_closed() -> boolean` (sync)

Example (interactive stdin):

```lua
local p = require("ward.process")

local child = p.cmd("cat"):spawn({ stdin = true, stdout = true })
local stdin = assert(child:stdin())
local out = assert(child:stdout_lines())

stdin:writeln("hello")
stdin:writeln("world")
stdin:close()

while true do
  local line, err = out:wait()
  if not line then break end
  print("echo:", line)
end

child:wait()
```

#### 6.3.4 `LineStream` userdata (line-by-line streaming)

Returned by `ProcChild:stdout_lines()` or `ProcChild:stderr_lines()`.

Note: Multiple coroutines reading the same `LineStream`/`ByteStream` will **compete** for data (load-balancing). Avoid creating multiple readers unless that is intended.

Methods:

- `stream:wait() -> line | nil, err`
  - `err` is `"eof"` when the stream ends.

This object follows Ward’s “awaitable” contract (`:wait()`), so it can be used with `async.select(...)`.

#### 6.3.5 `ByteStream` userdata (raw byte streaming)

Returned by `ProcChild:stdout_bytes()` or `ProcChild:stderr_bytes()`.

Note: Multiple coroutines reading the same `LineStream`/`ByteStream` will **compete** for data (load-balancing). Avoid creating multiple readers unless that is intended.

Methods:

- `stream:wait(n?) -> bytes | nil, err`
  - `n` defaults to 16384
  - returns a **bytes string** (binary-safe; may contain `\\0`)
  - `err` is `"eof"` when the stream ends.

Aliases:

- `stream:read(n?)` is the same as `stream:wait(n?)`.
- `stream(n?)` calls `stream:wait(n?)`.

This object follows Ward’s “awaitable” contract, so it can be used with `async.select(...)`.

Example (bytes):

```lua
local p = require("ward.process")

local child = p.cmd("sh", "-lc", "printf 'A\\0B'"):spawn({ stdout = true })
local bs = assert(child:stdout_bytes())
local chunk, err = bs:wait(3)
assert(chunk, err)

print("len:", #chunk)
print("bytes:", string.byte(chunk, 1, #chunk))

child:wait()
```

### 6.4 `Pipeline` userdata

Builder methods:

- `pl:pipefail(true|false) -> Pipeline` — if true, pipeline ok requires all steps succeed
- `pl:timeout(ms|duration_string|nil) -> Pipeline`
- `pl:pipe(cmd_or_pipeline) -> Pipeline`

Timeouts accept milliseconds (number) or human strings (`"500ms"`, `"2s"`, `"1m"`). Pass `nil` to clear.

Terminal operations:

- `pl:run() -> CmdResult`
- `pl:output() -> CmdResult`
- `pl:spawn(opts?) -> ProcChild`

`pl:spawn(opts?)` starts the pipeline and returns a `ProcChild` for streaming. The returned child refers to the **last stage** in the pipeline; use `child:pids()` to get all stage PIDs.

Operator:

- `pl | cmd` extends the pipeline

Example: pipe + capture

```lua
local p = process.cmd("cat", "README.md") | process.cmd("wc", "-l")
local r = p:output()
r:assert_ok()
print("lines:", r.stdout)
```

## 6.5 `ward.async` — tasks and channels

```lua
local async = require("ward.async")
```

### 6.5.1 Overview

Ward scripts run in an async-capable Lua runtime. Most operations that involve I/O (process execution, HTTP, timers) are implemented using async Rust internally, but can typically be called from Lua in a straightforward way because Ward drives the runtime for you.

`ward.async` provides **user-level concurrency primitives**:

- **Tasks**: run Lua functions concurrently via `async.spawn(...)`.
- **Channels**: communicate between tasks with bounded queues via `async.channel(...)`.
- **Await helpers**: a single awaitable contract (`:wait()` / `__call()`) used consistently by `async.await` and `async.select`.

These primitives are intended for local concurrency (I/O overlap, worker pools, fan-out/fan-in), not for CPU-parallel Lua execution.

### 6.5.2 Awaitables contract (important)

Ward uses a single user-facing await protocol:

- An **awaitable** is a userdata that implements `:wait()` (preferred) and may also implement `__call()` so it can be awaited via `a()`.
- `async.await(awaitable)` and `async.select(list)` operate on this protocol.
- `Task` and `Channel` are awaitables via `Task:wait()` and `Channel:wait()`.

This contract exists to keep concurrency composable: you can pass awaitables into `async.select` without “eagerly awaiting” them first.

### 6.5.3 Tasks

#### `async.spawn(fn, ...) -> Task`

Spawns a concurrent Lua task that runs `fn(...)`.

```lua
local t = async.spawn(function(x)
  return x + 1, "ok"
end, 41)
```

**Lifetime and cancellation semantics**

Tasks are **structured by default**: if the `Task` handle becomes unreachable and is garbage-collected (or dropped by losing scope), the underlying task may be aborted.

If you intentionally want a “fire-and-forget” task, call `t:detach()` to prevent abort-on-drop. Prefer structured tasks unless you have a clear reason to detach.

#### `Task:wait() -> ...`

Waits for the task to finish and returns the function’s return values.

```lua
local n, s = t:wait()
print(n, s) -- 42  ok
```

This makes `Task` compatible with the awaitables contract and with `async.select`.

Errors:

- Raises `"task already joined"` if `wait()` is called more than once.
- Raises `"cancelled"` if the underlying task was aborted (for example, because the handle was dropped without `detach()`).

#### `Task:cancel() -> boolean`

Requests task cancellation.

- Returns `true` if cancellation was requested.
- Returns `false` if the task is already finished (or already cancelled).

#### `Task:done() -> boolean`

Returns `true` when the task has finished.

#### `Task:detach() -> boolean`

Detaches the task from structured cancellation-on-drop/GC.

- Returns `true` after switching the task into detached mode.
- After detaching, dropping the `Task` handle will **not** abort the underlying task.

### 6.5.4 Channels

#### `async.channel(opts?) -> Channel`

Creates a bounded channel.

Accepted `opts`:

- `nil` (defaults apply)
- a number (capacity)
- a table: `{ capacity = N }`

If omitted, capacity defaults to **64**.

```lua
local ch = async.channel({ capacity = 16 })
```

#### `Channel:send(value) -> true | nil, err`

Async. Sends a value into the channel.

- Returns `true` on success.
- Returns `nil, "closed"` if the channel is closed.

#### `Channel:try_send(value) -> true | nil, err`

Sync. Attempts to send without waiting.

- Returns `true` on success.
- Returns `nil, "full"` if the buffer is full.
- Returns `nil, "closed"` if the channel is closed.

#### `Channel:wait() -> value | nil, err`

Async. Receives a value from the channel.

- Returns the value on success.
- Returns `nil, "closed"` after the sender is closed **and** the queue is drained.

This makes `Channel` compatible with the awaitables contract and with `async.select`.

Notes:

- `Channel:wait()` returns `nil, "closed"` only after the sender is closed **and** the queue is drained.
- If your channel can carry `nil`, always check `err` to distinguish a `nil` message from closure.

#### `Channel:try_recv() -> value | nil, err`

Sync. Attempts to receive without waiting.

- Returns the value on success.
- Returns `nil, "empty"` if no value is available.
- Returns `nil, "closed"` if the channel is disconnected.

Note: if another task is currently blocked in `wait()`, `try_recv()` may return `nil, "busy"` (implementation detail). Treat both `"empty"` and `"busy"` as retryable states.

#### `Channel:close() -> true`

Closes the **sender** side of the channel.

Important: `close()` does **not** discard queued items. Receivers can continue calling `wait()` until the channel is fully drained and then observe `nil, "closed"`.

### 6.5.5 Selecting across awaitables

#### `async.select(list) -> idx, ...`

Races multiple awaitables concurrently and returns the first one that completes.
`list` must be an array-like table of **userdata awaitables**. For convenience, Ward also accepts:

- `Task` userdata (waited via `:wait()`)
- `Channel` userdata (waited via `:wait()`)

The return value is:

- `idx` — the **1-based** index into `list` that completed first
- followed by that awaitable’s return values

Note: `async.select` cancels the non-winning internal *waiters* (the race participants), but it does not automatically cancel arbitrary underlying work unless that work is itself cancellation-aware. For example, if you race a long-running task against a timeout, the task will continue running unless you explicitly cancel it (or allow it to be aborted via structured drop/GC behavior).

Example: race a task against a timeout

```lua
local async = require("ward.async")
local time = require("ward.time")

local t = async.spawn(function()
  time.sleep(0.2)()
  return "task"
end)

local idx, v = async.select({ t, time.sleep(0.05) })
print("winner", idx, v)
```

### 6.5.6 `async.await(awaitable) -> ...`

Awaits a single awaitable userdata using the awaitables contract (`:wait()` preferred, or `__call()`).

This is mostly a convenience wrapper for readability and for writing higher-level helpers.

```lua
local async = require("ward.async")
local time = require("ward.time")

async.await(time.sleep(0.1))
```

### 6.5.7 Examples

#### Fan-out / fan-in

```lua
local async = require("ward.async")
local process = require("ward.process")

local workers = 4
local ch = async.channel({ capacity = 32 })

for i = 1, workers do
  async.spawn(function()
    local r = process.cmd("sh", "-lc", "echo worker=" .. i):output()
    ch:send({ i = i, ok = r.ok, out = r.stdout })
  end)
end

for _ = 1, workers do
  local msg = ch:wait()
  print(msg.i, msg.ok, msg.out)
end

ch:close()
```

#### Worker pool

See `samples/async.worker_pool.lua` for a complete runnable example.

### 6.6 Streaming example: watch a long-running command and react to new output

This pattern is common in bash/sh scripting (e.g., `tail -f ... | while read ...; do ...; done`).
In Ward, do it with `spawn()` and `stdout_lines()`:

```lua
local async = require("ward.async")
local p = require("ward.process")
local str = require("ward.helpers.string")

local NEEDLE = "PipeWire:Interface:Device"

local child = p.cmd("pw-mon", "-oap"):spawn({ stdout = true })
local out = assert(child:stdout_lines())

while true do
  local line, err = out:wait()
  if not line then break end

  -- Only reacts to *new* lines, not the initial process output snapshot.
  if str.contains(line, NEEDLE) then
    local r = p.cmd("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"):output()
    if r.ok then
      print(str.trim(r.stdout or ""))
    end
  end
end

child:wait()
```

---

## 7. `ward.time` — wall clock, parsing, durations, timers

```lua
local time = require("ward.time")
```

### 7.1 Wall clock

- `time.now() -> TimePoint`
- `time.now_table() -> table` — returns a table with timestamp components

### 7.2 Parsing

- `time.parse_rfc3339(s) -> TimePoint|nil`
- `time.parse_rfc2822(s) -> TimePoint|nil`
- `time.parse(s) -> TimePoint|nil` — best-effort parser

### 7.3 Construction

- `time.from_timestamp(seconds, nanos?) -> TimePoint`
- `time.utc(y, m, d, hh?, mm?, ss?, nanos?) -> TimePoint`

### 7.4 Durations

#### `time.duration(x) -> Duration`

Accepts:

- number: treated as seconds
- table: `{ days, hours, minutes, seconds, millis, micros }` (all optional)
- Duration userdata

Examples:

```lua
local d1 = time.duration(1.5)
local d2 = time.duration({ millis = 250 })
```

### 7.5 Monotonic time

- `time.instant_now() -> InstantPoint`

### 7.6 Timers (return awaitables)

These return userdata you must call `()` or `:wait()`.

- `time.sleep(duration) -> SleepAwaitable`
- `time.after(duration, callback?) -> AfterAwaitable`
- `time.interval(duration) -> IntervalTimer`
- `time.timeout(awaitable, duration) -> TimeoutAwaitable`

Examples:

```lua
-- sleep
time.sleep({ millis = 200 })()

-- after
local v = time.after({ seconds = 1 }, function() return "done" end)()
print(v)

-- interval
local it = time.interval({ seconds = 1 })
for _ = 1, 3 do
  print("tick", it())
end

-- timeout
local a = time.sleep({ seconds = 5 })
local ok, err = pcall(function()
  time.timeout(a, { millis = 200 })()
end)
print(ok, err)
```

### 7.7 Blocking sleep

- `time.sleep_blocking(duration) -> true`

---

## 8. `ward.term` — terminal utilities (prompting, ansi, progress)

```lua
local term = require("ward.term")
```

### 8.1 Input (returns awaitables)

All input helpers return an `InputAwaitable` userdata.

- `term.prompt(args) -> InputAwaitable` (returns `string|nil` when awaited)
- `term.confirm(args) -> InputAwaitable` (returns `boolean` when awaited)
- `term.password(args) -> InputAwaitable` (returns `string|nil` when awaited)
- `term.choose(args) -> InputAwaitable` (returns `string|nil` when awaited)

Awaitable methods:

- `a:wait() -> value`
- `a() -> value` (via `__call`)

#### `term.prompt{ question, default?, trim? }`

```lua
local name = term.prompt({ question = "Name", default = "guest" })()
print("hello", name)
```

#### `term.confirm{ question, default? }`

Accepted answers: `y/yes` and `n/no` (case-insensitive). Empty input returns the `default` if provided.

```lua
local ok = term.confirm({ question = "Continue?", default = false })()
if not ok then return end
```

#### `term.password{ prompt, trim? }`

Reads a line with no echo (TTY).

```lua
local secret = term.password({ prompt = "Password:" })()
```

#### `term.choose{ question, options, default? }`

`options` is an array-like table.

```lua
local choice = term.choose({
  question = "Pick one",
  options = { "dev", "staging", "prod" },
  default = "dev",
})()
print(choice)
```

### 8.2 Printing

- `term.print(...) -> true`
- `term.println(...) -> true`
- `term.eprint(...) -> true`
- `term.eprintln(...) -> true`

### 8.3 Screen control and tty

- `term.clear() -> true`
- `term.isatty(stream?) -> boolean` — `stream` can be `"stdout"` or `"stderr"`

### 8.4 `term.ansi` submodule

`term.ansi` is a table of ANSI escape-code strings you can concatenate into output.

Common style fields:

- `ansi.reset`
- `ansi.bold`, `ansi.dim`, `ansi.italic`, `ansi.underline`, `ansi.blink`, `ansi.reverse`, `ansi.hidden`, `ansi.strike`

Clear / cursor fields:

- `ansi.clear_line`, `ansi.clear_screen`, `ansi.home`

Colors (foreground):

- `ansi.black`, `ansi.red`, `ansi.green`, `ansi.yellow`, `ansi.blue`, `ansi.magenta`, `ansi.cyan`, `ansi.white`, `ansi.default`
- `ansi.bright_black`, `ansi.bright_red`, `ansi.bright_green`, `ansi.bright_yellow`, `ansi.bright_blue`, `ansi.bright_magenta`, `ansi.bright_cyan`, `ansi.bright_white`

Colors (background):

- `ansi.bg_black`, `ansi.bg_red`, `ansi.bg_green`, `ansi.bg_yellow`, `ansi.bg_blue`, `ansi.bg_magenta`, `ansi.bg_cyan`, `ansi.bg_white`, `ansi.bg_default`
- `ansi.bg_bright_black`, `ansi.bg_bright_red`, `ansi.bg_bright_green`, `ansi.bg_bright_yellow`, `ansi.bg_bright_blue`, `ansi.bg_bright_magenta`, `ansi.bg_bright_cyan`, `ansi.bg_bright_white`

Example:

```lua
local ansi = term.ansi
term.println(ansi.bold .. ansi.green .. "OK" .. ansi.reset)
```

### 8.5 `term.progress(args?) -> Progress`

Create a progress renderer for TTY output.

Constructor args (table):

- `total` (integer|nil)
- `message` (string|nil)
- `stream` (`"stdout"|"stderr"`, default `"stderr"`)

Progress methods (getter/setter style):

- `p:tick(delta?) -> nil` — increment by `delta` (default 1)
- `p:value(v?) -> integer|nil` — get current when called without args; set when `v` provided
- `p:total(t?) -> integer|nil` — get total when called without args; set when `t` provided
- `p:message(s?) -> string|nil` — get message when called without args; set when `s` provided
- `p:finish(final_msg?) -> true` — render final line + newline (TTY only)

Example:

```lua
local p = term.progress({ total = 10, message = "Working" })
for _ = 1, 10 do
  time.sleep({ millis = 150 })()
  p:tick(1)
end
p:finish("Done")
```

---

## 9. `ward.net` — HTTP requests and fetching

```lua
local net   = require("ward.net")
local http  = require("ward.net.http")
local fetch = require("ward.net.fetch")
```

`ward.net` groups network-related helpers. Today it exposes two submodules:

- `ward.net.http` — in-process HTTP requests via `reqwest`
- `ward.net.fetch` — higher-level “fetch into a file/dir” helpers

### 9.1 `ward.net.http` — HTTP request primitives

#### Functions

All functions below are **async** (implemented with `create_async_function`). Call them normally and receive a `HttpResponse` userdata.

- `http.get(url, opts?) -> HttpResponse`
- `http.delete(url, opts?) -> HttpResponse`
- `http.options(url, opts?) -> HttpResponse`
- `http.post(url, opts?) -> HttpResponse`
- `http.put(url, opts?) -> HttpResponse`

#### Options (`opts` table)

`opts` is optional. When omitted or not a table, defaults are applied.

- `query` (table) — query parameters.
  - Keys are strings.
  - Values must be `string`, `number`, `integer`, or `boolean` (they are converted to strings).
- `headers` (table) — header map: `string -> string`.
- `timeout` (number) — request timeout in **seconds** (float accepted). Must be positive and finite.
- `follow_redirects` (boolean, default `true`) — when enabled, redirects are followed (limited to 10).
- `allow_error` (boolean, default `false`) —
  - `false`: non-2xx responses raise a runtime error.
  - `true`: non-2xx responses are returned as `HttpResponse`.

Body options (used by `post` and `put`):

- `json` (any) — serializable Lua value encoded as JSON.
- `form` (table) — form fields `string -> string`.

If both `json` and `form` are present, JSON takes precedence.

#### `HttpResponse` userdata

Returned by `http.*` functions.

Fields (also available via methods):

- `resp.status` (integer) — HTTP status code.
- `resp.headers` (table) — header map `string -> string`.
  - Note: duplicate header names will be overwritten in the table (last one wins).
- `resp.body` (string|nil) — response body decoded as text.

Methods:

- `resp:is_ok() -> boolean` — true for 2xx.
- `resp:status() -> integer`
- `resp:headers() -> table`
- `resp:body() -> string|nil`

Examples:

```lua
local http = require("ward.net.http")

-- Basic GET
local r = http.get("https://example.com", { follow_redirects = true })
print(r.status)
print(r:is_ok())

-- Query + headers
local r2 = http.get("https://httpbin.org/get", {
  query = { q = "ward", page = 1, debug = true },
  headers = { ["User-Agent"] = "ward" },
  timeout = 10,
  allow_error = true,
})
print(r2.status)
print(r2:body())

-- POST JSON
local r3 = http.post("https://httpbin.org/post", {
  json = { hello = "world", n = 1 },
  headers = { ["Content-Type"] = "application/json" },
})
assert(r3:is_ok())
```

### 9.2 `ward.net.fetch` — fetch into a file/dir

`fetch` is for “download/checkout into a path” workflows that compose well with `ward.fs`.

#### `fetch.url(url, opts?) -> FetchResponse`

Downloads the response body as bytes into a file (streaming), then returns metadata.

Options (`opts` table):

- `method` (string, default `"GET"`) — one of: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`.
- `headers` (table) — header map `string -> string`.
- `timeout` (number) — request timeout in **seconds**.
- `follow_redirects` (boolean, default `true`) — redirects are followed (limited to 10).
- `into` (string|nil) — destination file path.
  - If omitted, Ward creates a unique file path under the OS temp directory.
- `max_bytes` (integer|number|nil) — maximum allowed response size.
  - Values `<= 0` disable the limit.
  - If the limit is exceeded, Ward removes the partial file and returns `ok=false` with `status=413` and `path=nil`.

`FetchResponse` userdata fields/methods:

- `resp.ok` / `resp:is_ok() -> boolean`
- `resp.status` / `resp:status() -> integer` — HTTP status code (or `413` if `max_bytes` exceeded).
- `resp.path` / `resp:path() -> string|nil` — destination path.
- `resp.size` / `resp:size() -> integer` — bytes written.

Example:

```lua
local fetch = require("ward.net.fetch")
local fs    = require("ward.fs")

local r = fetch.url("https://example.com/file.tar.gz", {
  into = "./downloads/file.tar.gz",
  max_bytes = 50 * 1024 * 1024,
})

if not r.ok then
  error("fetch failed: status=" .. tostring(r.status))
end

print("saved to", r.path, "bytes", r.size)
assert(fs.is_file(r.path))
```

#### `fetch.git(url, opts?) -> FetchResponse`

Clones a Git repository into a directory (using the external `git` command), then optionally checks out a revision.

Notes:

- Requires `git` to be installed and discoverable in `PATH`.
- `git` stdout/stderr are suppressed; use `ok`/`status` to handle errors.

Options (`opts` table):

- `into` (string|nil) — destination directory.
  - If omitted, Ward creates a unique directory under the OS temp directory.
- `depth` (integer|nil) — shallow clone depth (must be > 0). Defaults to `1`.
- `filter_blobs` (boolean, default `true`) — when true, uses `--filter=blob:none`.
- `branch` (string|nil) — clone a specific branch.
- `tag` (string|nil) — clone a specific tag (used as `--branch <tag>`).
  - If both `branch` and `tag` are set, `branch` takes precedence.
- `recursive` (boolean, default `false`) — when true, uses `--recurse-submodules`.
- `rev` (string|nil) — if set, runs `git checkout <rev>` after cloning.
- `timeout` (number|nil) — command timeout in **seconds**.
- `max_bytes` (integer|number|nil) — maximum allowed on-disk size for the cloned directory.
  - If exceeded, Ward removes the directory and returns `ok=false` with `status=413` and `path=nil`.

On success, `FetchResponse.status` is `0` and `ok=true`. On failure, `status` is the `git` exit code.

Example:

```lua
local fetch = require("ward.net.fetch")

local r = fetch.git("https://github.com/user/repo.git", {
  into = "./vendor/repo",
  depth = 1,
  rev = "v1.2.3",
  recursive = false,
  timeout = 120,
})

if not r.ok then
  error("git fetch failed: exit=" .. tostring(r.status))
end

print("checked out into", r.path, "bytes", r.size)
```

---

## 10. `ward.convert` — serialization formats

```lua
local convert = require("ward.convert")
local json = require("ward.convert.json")
local yaml = require("ward.convert.yaml")
local toml = require("ward.convert.toml")
local ini  = require("ward.convert.ini")
```

Common patterns:

- `encode(value) -> string`
- `decode(string) -> value`

Example (JSON):

```lua
local json = require("ward.convert.json")
local s = json.encode({ a = 1, b = { true, false } })
local t = json.decode(s)
```

---

## 11. `ward.helpers` — small utility functions

```lua
local helpers = require("ward.helpers")
local retry   = require("ward.helpers.retry")
local n = require("ward.helpers.number")
local s = require("ward.helpers.string")
local t = require("ward.helpers.table")
```

### 11.1 `helpers.number`

Includes numeric predicates and helpers.

Notable behavior:

- `is_number` treats both Lua integers and floats as numbers.

### 11.2 `helpers.string`

Includes regex/string helpers, capture utilities, etc.

### 11.3 `helpers.table`

Includes table utilities.

### 11.4 `helpers.retry`

helpers.retry implements an async retry loop for functions that may intermittently fail.

#### `retry.run(fn, opts?) -> any`

Calls fn() and returns its result. If fn() errors, Ward retries until success or the attempt limit is reached.

Options (opts table):

- attempts (integer, default 3) — total attempts (minimum 1)
- delay_ms (integer, default 100) — base delay between retries
- backoff (number, default 2.0) — multiplier applied to the delay after each failed attempt (minimum 1.0)
- max_delay_ms (integer|nil) — optional cap on the delay
- jitter (boolean, default false) — randomize delay to reduce thundering herd
- jitter_ratio (number, default 0.2) — maximum relative jitter, clamped to 0..1

Example:

```lua
local retry = require("ward.helpers.retry")
local net = require("ward.net.http")

local res = retry.run(function()
    ...
end, { attempts = 5, delay_ms = 200, backoff = 2.0, jitter = true })

print("ok:", res.status)
```

---

## 12. `ward.crypto` — hashing and checksums

```lua
local crypto = require("ward.crypto")
```

Byte-string functions (Lua strings are binary-safe):

- `crypto.sha256(bytes) -> string` (hex)
- `crypto.sha1(bytes) -> string` (hex)
- `crypto.md5(bytes) -> string` (hex)

File functions (async, streamed):

- `crypto.sha256_file(path) -> string` (hex)
- `crypto.sha1_file(path) -> string` (hex)
- `crypto.md5_file(path) -> string` (hex)

Examples:

```lua
local crypto = require("ward.crypto")

local digest = crypto.sha256("abc")
print("sha256(abc) =", digest)

local file_digest = crypto.sha256_file("Cargo.toml")
print("sha256(Cargo.toml) =", file_digest)
```

## 13. `ward.log` — logging

```lua
local log = require("ward.log")
log.info("hello", "world")
log.trace(...)
log.debug(...)
log.warn(...)
log.error(...)
log.fatal(...)
```

Ward log is intentionally minimal; use it for script-friendly logs.

---

## 14. `ward.host` — platform and host resources

```lua
local host = require("ward.host")
local platform = require("ward.host.platform")
local resources = require("ward.host.resources")
```

### 14.1 `host.platform`

Platform inspection helpers (OS, arch, etc.).

### 14.2 `host.resources`

Resource inspection (memory, CPU) for the host process.

---

## 15. `ward.lifecycle` — shutdown hooks and signals

```lua
local lifecycle = require("ward.lifecycle")
```

Lifecycle provides:

- Shutdown request detection (signals, cancellation)
- Hooks you can register to run before exit

Common pattern:

```lua
local lifecycle = require("ward.lifecycle")

lifecycle.on_shutdown(function(reason)
  -- flush files, cleanup temp dirs
end)
```

---

## 16. Examples (shell replacement style)

### 16.1 "set -e" style: assert on failures

```lua
local process = require("ward.process")

process.cmd("git", "rev-parse", "--is-inside-work-tree")
  :output()
  :assert_ok("not a git repo")
```

### 16.2 Download to temp dir and process

```lua
local fs = require("ward.fs")
local process = require("ward.process")
local term = require("ward.term")
local time = require("ward.time")

local dir = fs.tempdir("ward-")
term.println("tmp:", dir)

-- Example pipeline
local p = process.cmd("printf", "hello\nworld\n") | process.cmd("wc", "-l")
local r = p:output()
r:assert_ok()
term.println("lines:", r.stdout)

-- progress demo
local prog = term.progress({ total = 5, message = "Working" })
for _ = 1, 5 do
  time.sleep({ millis = 200 })()
  prog:tick(1)
end
prog:finish("Done")
```

## 17. `ward.template.minijinja` — templates

MiniJinja is a fast, Rust-native Jinja2-like template engine.

```lua
local tpl = require("ward.template.minijinja")
```

### 17.1 `minijinja.render(template, context, opts?) -> string`

Render a template string using the given `context` (Lua table).

```lua
local out = tpl.render("Hello {{ user.name }}!", {
  user = { name = "Ward" }
})
print(out) -- Hello Ward!
```

### 17.2 `minijinja.render_async(template, context, opts?) -> string`

Same as `render`, but runs the render on a blocking thread so it will not block the async runtime.

```lua
local out = tpl.render_async("{{ n }}", { n = 42 })
print(out)
```

### 17.3 `minijinja.render_file(path, context, opts?) -> string`

Read a file from `path` and render its contents as a template.

```lua
local out = tpl.render_file("./hello.tmpl", { name = "world" })
print(out)
```

### 17.4 `minijinja.render_file_async(path, context, opts?) -> string`

Same as `render_file`, but runs on a blocking thread.

### 17.5 Options

All functions accept an optional `opts` table:

- `undefined`: one of `"strict"`, `"lenient"`, `"chainable"` (default: `"strict"`)
- `trim_blocks`: boolean (default: `false`)
- `lstrip_blocks`: boolean (default: `false`)
- `keep_trailing_newline`: boolean (default: `false`)
- `auto_escape`: boolean (default: `false`)
- `loader`: table configuring `{% include %}` / `{% import %}` resolution
  - `paths`: array of strings; additional search paths for templates

Example:

```lua
local out = tpl.render_file("./templates/main.j2", {
  title = "Hello",
  items = { "a", "b", "c" },
}, {
  undefined = "strict",
  trim_blocks = true,
  lstrip_blocks = true,
  loader = {
    paths = { "./templates" },
  },
})
print(out)
```

