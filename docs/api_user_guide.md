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
- “bytes string” means a Lua string whose contents are raw bytes (binary-safe).
- Paths are typically accepted as strings (and in most ward.fs APIs also as ward.fs.path objects).
- Most option tables are optional; when omitted, defaults apply.

---

## 3. `ward.env` — environment and PATH tools

Ward uses an environment overlay:

- `env.set` / `env.unset` / `env.clear` modify Ward’s overlay only (they do not mutate the process-global OS environment).
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

### 3.2 `env.set(key, value) -> true`

Set an environment variable in the Ward overlay.

```lua
env.set("FOO", "bar")
```

### 3.3 `env.unset(key) -> true`

Remove an environment variable (from the overlay).

```lua
env.unset("FOO")
```

### 3.4 `env.clear() -> true`

Clears all overlay modifications (restores the effective environment back to the base process environment).

### 3.5 `env.list() -> table`

Returns a table of the effective environment (base process env with overlay applied).

Example (print all):

```lua
local t = env.list()
for k, v in pairs(t) do
  print(k, v)
end
```

### 3.6 `env.is_exists(key) -> boolean`

Returns `true` if variable exists in the effective environment.

```lua
if env.is_exists("CI") then
  print("running in CI")
end
```

### 3.7 `env.hostname() -> string`

Returns hostname.

### 3.8 `env.which(name) -> string|nil`

Searches `PATH` (and Windows `PATHEXT`) for an executable.

```lua
local git = env.which("git")
assert(git, "git not found")
```

### 3.9 `env.is_in_path(path_or_name) -> boolean`

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
assert(fs.mkdir(p:dirname(), { recursive = true, force = true }))
fs.write(p, "hello\n")
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

#### `fs.mkdir(path, opts?) -> boolean`

Create directory.

Options:

- `recursive` (boolean, default `false`)
- `mode` (number, unix-only; default `0o755`)
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

### 5.1 `io.read_all(opts?) -> string`

Reads all remaining stdin into a string.

Optional `opts`:

- `max_bytes` (number|integer) — if provided, fails when stdin exceeds this limit.

```lua
local s = io.read_all()

-- hard cap (1 MiB)
local s2 = io.read_all({ max_bytes = 1024 * 1024 })
```

### 5.2 `io.read_line() -> string|nil`

Reads one line from stdin.

- Returns `nil` on EOF.

```lua
local line = io.read_line()
if line == nil then return end
print("got:", line)
```

### 5.3 `io.read_lines() -> function`

Returns an iterator-like function. Each call reads one line from stdin and returns `string|nil` (nil on EOF).

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

### 6.4 `Pipeline` userdata

Builder methods:

- `pl:pipefail(true|false) -> Pipeline` — if true, pipeline ok requires all steps succeed
- `pl:pipe(cmd_or_pipeline) -> Pipeline`

Terminal operations:

- `pl:run() -> CmdResult`
- `pl:output() -> CmdResult`

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

+print("ok:", res.status)
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
