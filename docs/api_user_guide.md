# Ward API user guide

This document is a user-facing reference for the Ward Lua runtime: modules, functions, userdata types, and common patterns.

> Scope: this guide is written against the `ward-4` codebase.

---

## 1. Mental model

Ward is a Lua runtime designed for writing scripts that feel like bash/sh, but with:

- Explicit modules (via `require("ward.*")`)
- Async-capable primitives (I/O, processes, timers)
- Stronger typing at the API boundary (option tables, structured results)

### 1.1 Importing

You can import either the root module or submodules:

```lua
local ward = require("ward")
local fs   = require("ward.fs")
local io   = require("ward.io")
local proc = require("ward.process")
local time = require("ward.time")
local term = require("ward.term")
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
- “bytes string” means a Lua string whose contents are raw bytes (binary-safe).
- Paths are typically accepted as strings.
- Most option tables are optional; when omitted, defaults apply.

---

## 3. `ward.env` — environment and PATH tools

```lua
local env = require("ward.env")
```

### 3.1 `env.get(key, default?) -> string|nil`
Get environment variable `key`. If missing (or `key` is empty), returns `default`.

```lua
local home = env.get("HOME")
local port = env.get("PORT", "8080")
```

### 3.2 `env.set(key, value) -> true`
Set an environment variable.

```lua
env.set("FOO", "bar")
```

### 3.3 `env.unset(key) -> true`
Remove an environment variable.

```lua
env.unset("FOO")
```

### 3.4 `env.list() -> table`
Returns a table of the current process environment.

Example (print all):

```lua
local t = env.list()
for k, v in pairs(t) do
  print(k, v)
end
```

### 3.5 `env.is_exists(key) -> boolean`
Returns `true` if variable exists.

```lua
if env.is_exists("CI") then
  print("running in CI")
end
```

### 3.6 `env.hostname() -> string`
Returns hostname.

### 3.7 `env.which(name) -> string|nil`
Searches `PATH` (and Windows `PATHEXT`) for an executable.

```lua
local git = env.which("git")
assert(git, "git not found")
```

### 3.8 `env.is_in_path(path_or_name) -> boolean`
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

#### `fs.mkdir(path, opts?) -> boolean`
Create directory.

Options:
- `recursive` (boolean, default `false`)
- `mode` (number, unix-only; default `0o644`)
- `force` (boolean, default `false`) — treat “already exists” as success

```lua
fs.mkdir("build", { recursive = true, mode = 0o755 })
```

#### `fs.rm(path, opts?) -> boolean`
Remove file or directory.

Options:
- `recursive` (boolean, default `false`) — required for directories
- `force` (boolean, default `false`) — treat missing-path as success

```lua
fs.rm("build", { recursive = true, force = true })
```

#### `fs.unlink(path, opts?) -> boolean`
Remove a file (like `rm -f file`).

### 4.5 Permissions and links

- `fs.chmod(path, mode) -> boolean`
- `fs.chown(path, uid, gid) -> boolean` (Unix)
- `fs.rename(from, to) -> boolean`
- `fs.link(from, to) -> boolean` (hard link)
- `fs.symlink(from, to) -> boolean` (symbolic link)

### 4.6 Timestamps

#### `fs.touch(path, opts?) -> boolean`
Create file if missing and update timestamps.

Options:
- `force` (boolean, default `false`)
- `recursive` (boolean, default `false`) — create parent directories first

```lua
fs.touch("logs/app.log", { recursive = true })
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

#### `fs.write(path, data, opts?) -> boolean`
Write data to a file.

Options (selected):
- `mode` (`"overwrite"|"append"|"prepend"|"binary"`, default `"overwrite"`)
- `append` (boolean, optional convenience; equivalent to `mode="append"`)
- `binary` (boolean, default `false`) — convert `data` as bytes
- `force` (boolean, default `false`)

Notes:
- Ward does not automatically create parent directories; combine with `fs.mkdir(fs.dirname(path), {recursive=true})`.

```lua
fs.write("out.txt", "hello\n")
fs.write("out.txt", "more\n", { mode = "append" })
```

### 4.8 Copy and move

- `fs.copy(from, to, opts?) -> boolean`
- `fs.move(from, to, opts?) -> boolean`

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

### 5.1 `io.read_all() -> string`
Reads all remaining stdin into a string.

```lua
local s = io.read_all()
```

### 5.2 `io.read_line() -> string|nil`
Reads one line from stdin.

- Returns `nil` on EOF.

```lua
local line = io.read_line()
if line == nil then return end
print("got:", line)
```

### 5.3 `io.read_lines() -> table`
Reads all remaining lines and returns an array-like table.

```lua
for _, line in ipairs(io.read_lines()) do
  print(line)
end
```

### 5.4 Output

- `io.write_stdout(data) -> true`
- `io.write_stderr(data) -> true`
- `io.flush_stdout() -> true`
- `io.flush_stderr() -> true`

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

```lua
local cmd = process.cmd("git", "status", "--porcelain")
```

#### `process.sh(script) -> Cmd`
Run a shell fragment using the platform default shell:

- Unix: `sh -lc <script>`
- Windows: `cmd /C <script>`

```lua
local cmd = process.sh("echo $HOME")
```

### 6.2 `ProcResult` userdata (result of run/output)

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

Builder methods (fluent):

- `cmd:cwd(path) -> Cmd`
- `cmd:env(key, value) -> Cmd`
- `cmd:envs(tbl) -> Cmd` (reads key/value pairs)
- `cmd:timeout(ms) -> Cmd`
- `cmd:stdin(data) -> Cmd` (bytes string)
- `cmd:stdin_file(path) -> Cmd`
- `cmd:stderr_to_stdout(true|false) -> Cmd`
- `cmd:pipe(other_cmd_or_pipeline) -> Pipeline`

Terminal operations:

- `cmd:run() -> ProcResult` — inherits stdio
- `cmd:output() -> ProcResult` — captures stdout/stderr

Pipeline operator:

- `cmd1 | cmd2` produces a `Pipeline` (via `__bor`).

Example:

```lua
local r = process.cmd("git", "rev-parse", "HEAD"):output()
r:assert_ok()
print(r.stdout)
```

### 6.4 `Pipeline` userdata

Builder methods:

- `pl:pipefail(true|false) -> Pipeline` — if true, pipeline ok requires all steps succeed
- `pl:pipe(cmd_or_pipeline) -> Pipeline`

Terminal operations:

- `pl:run() -> ProcResult`
- `pl:output() -> ProcResult`

Operator:

- `pl | cmd` extends the pipeline

Example: pipe + capture

```lua
local p = process.cmd("cat", "README.md") | process.cmd("wc", "-l")
local r = p:output()
r:assert_ok()
print("lines:", r.stdout)
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

## 9. `ward.net` — HTTP and fetching

```lua
local net = require("ward.net")
local http = require("ward.net.http")
local fetch = require("ward.net.fetch")
```

### 9.1 `ward.net.fetch`
Fetch provides higher-level helpers for downloading.

- `fetch.url(url, opts?) -> bytes string|ProcResult` (depending on API)

> Note: refer to the module’s exported function list in your source tree; the API is intentionally small and is designed to compose with `fs.write`.

### 9.2 `ward.net.http`
HTTP provides request/response primitives.

> Note: refer to the module’s exported function list in your source tree; the API is intentionally small and focuses on correctness over breadth.

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

---

## 12. `ward.log` — logging

```lua
local log = require("ward.log")
```

Ward log is intentionally minimal; use it for script-friendly logs.

---

## 13. `ward.host` — platform and host resources

```lua
local host = require("ward.host")
local platform = require("ward.host.platform")
local resources = require("ward.host.resources")
```

### 13.1 `host.platform`
Platform inspection helpers (OS, arch, etc.).

### 13.2 `host.resources`
Resource inspection (memory, CPU) for the host process.

---

## 14. `ward.lifecycle` — shutdown hooks and signals

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

## 15. Examples (shell replacement style)

### 15.1 "set -e" style: assert on failures

```lua
local process = require("ward.process")

process.cmd("git", "rev-parse", "--is-inside-work-tree")
  :output()
  :assert_ok("not a git repo")
```

### 15.2 Download to temp dir and process

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

