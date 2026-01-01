- [Constructors](#constructors)
  - [Process middleware (command wrappers)](#process-middleware-command-wrappers)
- [`CmdResult` userdata (result of run/output)](#cmdresult-userdata-result-of-runoutput)
- [`Cmd` userdata](#cmd-userdata)
  - [`Cmd:spawn(opts?) -> ProcChild`](#cmdspawnopts---procchild)
  - [`ProcChild` userdata](#procchild-userdata)
  - [`ProcStdin` userdata (interactive stdin)](#procstdin-userdata-interactive-stdin)
  - [`LineStream` userdata (line-by-line streaming)](#linestream-userdata-line-by-line-streaming)
  - [`ByteStream` userdata (raw byte streaming)](#bytestream-userdata-raw-byte-streaming)
- [`Pipeline` userdata](#pipeline-userdata)
- [`ward.async` - tasks and channels](#wardasync---tasks-and-channels)
  - [Overview](#overview)
  - [Awaitables contract (important)](#awaitables-contract-important)
  - [Tasks](#tasks)
  - [Channels](#channels)
  - [Selecting across awaitables](#selecting-across-awaitables)
  - [`async.await(awaitable) -> ...`](#asyncawaitawaitable---)
  - [Examples](#examples)
- [Streaming example: watch a long-running command and react to new output](#streaming-example-watch-a-long-running-command-and-react-to-new-output)

```lua
local process = require("ward.process")
```

### Constructors

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
- Ward requests shutdown, unwinds execution, runs shutdown handlers, and
then the CLI exits with the given status.
- `code` defaults to `0`. Negative values are coerced to `1`. Values above
`i32::MAX` are clamped.

Use this for early returns from scripts that still need cleanup handlers to run.

#### `process.shell_defaults(opts?) -> table`

Set or read defaults that mimic common `set -euo pipefail` toggles:

- `pipefail` (boolean) - when true, new commands/pipelines treat any non-zero
step as failure by default.
- `timeout` (number|string|nil) - pipeline-level timeout for new
commands/pipelines. Accepts integers (ms) or human strings (`500ms`,
`2s`, `1m`). `nil` clears the default.

Returns a table with the active defaults. Example:

```lua
local process = require("ward.process")

-- Enable pipefail + a 30s default timeout for all new cmds/pipelines.
process.shell_defaults({ pipefail = true, timeout = "30s" })
```

#### Process middleware (command wrappers)

Ward exposes a small, composable *middleware stack* that is applied to every process spawn:

- `Cmd:run()` / `Cmd:output()` / `Cmd:spawn()`
- `Pipeline:run()` / `Pipeline:output()` / `Pipeline:spawn()`

This is the intended core building block for `wardlib` wrappers (for example, `sudo` / `doas`) so users do not need to repeat a prefix in every `process.cmd(...)` call.

**Semantics**

- Middleware is **task-local** (per `ward.async` task). When you `async.spawn(...)`, the current middleware stack is inherited by the new task.
- Middleware runs **synchronously** right before the OS process is spawned.
- Each middleware is a Lua function `mw(spec)`.
  - It may *mutate* the passed `spec` table and return `nil`, or
  - return a new spec table.

**Spec contract**

The `spec` table contains the following fields (future versions may add more):

- `spec.argv` (table) - array of strings: `{ program, arg1, arg2, ... }` (must remain non-empty)
- `spec.cwd` (string|nil)
- `spec.env` (table) - string-to-string map of per-command overrides

**API**

- `process.push_middleware(fn) -> integer` - push `fn` to the current task's stack (returns new depth)
- `process.pop_middleware() -> boolean` - pop the last middleware (returns `false` if the stack is empty)

**Example: wrap commands with `sudo`/`doas` (non-interactive by default)**

```lua
local process = require("ward.process")

local function with_middleware(mw, body)
  process.push_middleware(mw)
  local ok, err = pcall(body)
  process.pop_middleware()
  if not ok then error(err) end
end

local function sudo_middleware(opts)
  opts = opts or {}
  local tool = opts.tool or "sudo"     -- or "doas"
  local non_interactive = opts.non_interactive ~= false

  return function(spec)
    local argv = spec.argv or {}
    if argv[1] == tool then return nil end

    local out = { tool }
    if non_interactive then
      table.insert(out, "-n")
    end
    table.insert(out, "--")
    for i = 1, #argv do table.insert(out, argv[i]) end
    spec.argv = out
    return nil
  end
end

-- If you want password prompting, pre-auth once with inherited stdio.
-- (Captured `output()` calls cannot safely prompt.)
process.cmd("sudo", "-v"):run():assert_ok("sudo -v failed")

with_middleware(sudo_middleware({ tool = "sudo" }), function()
  process.cmd("id", "-u"):run():assert_ok()
end)
```

### `CmdResult` userdata (result of run/output)

Fields:

- `result.ok` (boolean)
- `result.code` (integer)
- `result.signal` (integer|nil)
- `result.stdout` (bytes string|nil)
- `result.stderr` (bytes string|nil)
- `result.steps` (table of integers) - per-step exit codes

Methods:

- `result:is_ok() -> boolean`
- `result:assert_ok(msg?) -> ()` - throws a Lua error if not ok

Example:

```lua
local r = process.cmd("echo", "hi"):output()
print(r.ok, r.code)
print(r.stdout)
r:assert_ok("echo failed")
```

### `Cmd` userdata

In addition to one-shot execution (`run` / `output`), `Cmd` supports spawning
long-running processes and streaming their stdio.

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

- `cmd:stdin(data)`, `cmd:stdin_file(path)`, and `cmd:stdin_null()` configure
stdin for `run()` / `output()` / `spawn()`. This is **one-shot** input: Ward
will feed the configured input (or connect stdin to the file/null stream)
and then close stdin.
- `cmd:stdin(v)` accepts only a bytes string, or `nil`/`false` to reset to
inherited stdin. Other values raise an error.
- `cmd:stdin_file(path)` fails at spawn-time if the file cannot be opened.
- `cmd:stderr_to_stdout(true)` merges stderr into stdout **best-effort**.
Ordering may differ from shell `2>&1`. In capture mode (`output()`), merged
data is returned in `stdout` and `stderr` is `nil`.

- `cmd:timeout(...)` accepts whole seconds (number) or human strings like
`"500ms"`, `"2s"`, `"1m"`. Pass `nil` to clear.

- `cmd:stdin_null()` sets stdin to a closed stream (equivalent to shell `< /dev/null`).
- `cmd:pipe(other_cmd_or_pipeline) -> Pipeline`

Terminal operations:

- `cmd:run() -> CmdResult` - inherits stdio
- `cmd:output() -> CmdResult` - captures stdout/stderr

Pipeline operator:

- `cmd1 | cmd2` produces a `Pipeline` (via `__bor`).

Example:

```lua
local r = process.cmd("git", "rev-parse", "HEAD"):output()
r:assert_ok()
print(r.stdout)
```

#### `Cmd:spawn(opts?) -> ProcChild`

Spawns a long-running child process and returns a `ProcChild` handle. This is
used for:

- Streaming stdout/stderr incrementally (lines or raw bytes)
- Interactive processes (writing to stdin)
- Long-running daemons/subscriptions (e.g., `pw-mon`, `tail -f`, etc.)

`opts` is an optional table:

- `stdin`  (boolean or `"pipe"|"inherit"|"null"`)
  - `true` / `"pipe"`: pipe stdin so Lua can write via `ProcChild:stdin()`
  - `false` / `"inherit"`: inherit parent stdin
  - `"null"`: connect stdin to a closed stream
  - Default: inferred. If you set `cmd:stdin(data)`, Ward pipes stdin to feed
  the bytes; if you set `cmd:stdin_null()`, stdin is null; if you set
  `cmd:stdin_file(path)`, Ward pre-opens the file and uses it as stdin.
  Otherwise stdin defaults to inherited stdin unless you request piping.

- `stdout` (boolean or `"pipe"|"inherit"|"null"`, default `true`)
  - `true` / `"pipe"`: pipe stdout so you can stream it
  - `false` / `"inherit"`: inherit parent stdout
  - `"null"`: discard stdout

- `stderr` (boolean or `"pipe"|"inherit"|"null"`)
  - `true` / `"pipe"`: pipe stderr so you can stream it
  - `false` / `"inherit"`: inherit parent stderr
  - `"null"`: discard stderr
  - Default: `true` when `cmd:stderr_to_stdout(true)` is set, otherwise
`false` (`"inherit"`).

Important:

- If you call `cmd:stderr_to_stdout(true)`, Ward merges stderr into the stdout
stream (similar to `2>&1`). In this case, stderr is not available separately
and must be read from stdout. Ordering is best-effort and may not exactly
match OS-level `2>&1` interleaving.
- Choose either one-shot stdin via `cmd:stdin(...)` / `cmd:stdin_file(...)` /
`cmd:stdin_null()`, or interactive stdin via `spawn({ stdin = true })` +
`ProcChild:stdin()`. Combining `cmd:stdin(...)` / `cmd:stdin_file(...)` with
`spawn({ stdin = true })` is invalid and raises an error.

Example (spawn + line streaming):

```lua
local p = require("ward.process")

local child = p.cmd("sh", "-lc", "printf 'a\\nb\\n' && sleep 1")
    :spawn({ stdout = true })
local out = assert(child:stdout_lines())

while true do
  local line, err = out:wait()
  if not line then break end
  print("line:", line)
end

child:wait()
```

#### `ProcChild` userdata

Represents a spawned child process.

Methods:

- `child:pid() -> integer`
- `child:pids() -> table` - array of PIDs for all stages (for pipelines)
- `child:stdin() -> ProcStdin | nil, err`
  - Returns `nil, "not_piped"` if stdin is not piped.
- `child:stdout_lines() -> LineStream | nil, err`
  - Returns `nil, "not_piped"` if stdout is not piped.
  - Returns `nil, "mode_conflict"` if stdout was already opened as bytes.
  - You may call this multiple times; each handle reads from the same
  underlying pipe.
- `child:stderr_lines() -> LineStream | nil, err`
  - Returns `nil, "not_piped"` if stderr is not piped.
  - Returns `nil, "merged"` if stderr was merged into stdout via `cmd:stderr_to_stdout(true)`.
  - Returns `nil, "mode_conflict"` if stderr was already opened as bytes.
  - You may call this multiple times; each handle reads from the same
  underlying pipe.
- `child:stdout_bytes() -> ByteStream | nil, err`
  - Returns `nil, "not_piped"` if stdout is not piped.
  - Returns `nil, "mode_conflict"` if stdout was already opened as lines.
  - You may call this multiple times; each handle reads from the same
  underlying pipe.
- `child:stderr_bytes() -> ByteStream | nil, err`
  - Returns `nil, "not_piped"` if stderr is not piped.
  - Returns `nil, "merged"` if stderr was merged into stdout via `cmd:stderr_to_stdout(true)`.
  - Returns `nil, "mode_conflict"` if stderr was already opened as lines.
  - You may call this multiple times; each handle reads from the same
  underlying pipe.
- `child:kill() -> boolean` (async)
- `child:wait() -> CmdResult` (async)

`ProcChild:wait()` returns a `CmdResult` with:

- `ok`, `code`, `signal`
- `stdout`/`stderr` are typically `nil` because streaming consumption is
incremental, not captured.

#### `ProcStdin` userdata (interactive stdin)

Returned by `ProcChild:stdin()` when stdin is piped.

Methods:

- `stdin:write(bytes_string) -> true | nil, err` (async)
- `stdin:writeln(string) -> true | nil, err` (async) - writes string + `\\n`
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

#### `LineStream` userdata (line-by-line streaming)

Returned by `ProcChild:stdout_lines()` or `ProcChild:stderr_lines()`.

Note: Multiple coroutines reading the same `LineStream`/`ByteStream` will
**compete** for data (load-balancing). Avoid creating multiple readers unless
that is intended.

Methods:

- `stream:wait() -> line | nil, err`
  - `err` is `"eof"` when the stream ends.

This object follows Ward's "awaitable" contract (`:wait()`), so it can be
used with `async.select(...)`.

#### `ByteStream` userdata (raw byte streaming)

Returned by `ProcChild:stdout_bytes()` or `ProcChild:stderr_bytes()`.

Note: Multiple coroutines reading the same `LineStream`/`ByteStream` will
**compete** for data (load-balancing). Avoid creating multiple readers unless
that is intended.

Methods:

- `stream:wait(n?) -> bytes | nil, err`
  - `n` defaults to 16384
  - returns a **bytes string** (binary-safe; may contain `\\0`)
  - `err` is `"eof"` when the stream ends.

Aliases:

- `stream:read(n?)` is the same as `stream:wait(n?)`.
- `stream(n?)` calls `stream:wait(n?)`.

This object follows Ward's "awaitable" contract, so it can be used with `async.select(...)`.

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

### `Pipeline` userdata

Builder methods:

- `pl:pipefail(true|false) -> Pipeline` - if true, pipeline ok requires all
steps succeed
- `pl:timeout(ms|duration_string|nil) -> Pipeline`
- `pl:pipe(cmd_or_pipeline) -> Pipeline`

Timeouts accept whole seconds (number) or human strings (`"500ms"`, `"2s"`,
`"1m"`). Pass `nil` to clear.

Terminal operations:

- `pl:run() -> CmdResult`
- `pl:output() -> CmdResult`
- `pl:spawn(opts?) -> ProcChild`

`pl:spawn(opts?)` starts the pipeline and returns a `ProcChild` for streaming.
The returned child refers to the **last stage** in the pipeline; use
`child:pids()` to get all stage PIDs.

Operator:

- `pl | cmd` extends the pipeline

Example: pipe + capture

```lua
local p = process.cmd("cat", "README.md") | process.cmd("wc", "-l")
local r = p:output()
r:assert_ok()
print("lines:", r.stdout)
```

## `ward.async` - tasks and channels

```lua
local async = require("ward.async")
```

### Overview

Ward scripts run in an async-capable Lua runtime. Most operations that involve
I/O (process execution, HTTP, timers) are implemented using async Rust
internally, but can typically be called from Lua in a straightforward
way because Ward drives the runtime for you.

`ward.async` provides **user-level concurrency primitives**:

- **Tasks**: run Lua functions concurrently via `async.spawn(...)`.
- **Channels**: communicate between tasks with bounded queues via `async.channel(...)`.
- **Await helpers**: a single awaitable contract (`:wait()` / `__call()`) used
consistently by `async.await` and `async.select`.

These primitives are intended for local concurrency (I/O overlap, worker pools,
fan-out/fan-in), not for CPU-parallel Lua execution.

### Awaitables contract (important)

Ward uses a single user-facing await protocol:

- An **awaitable** is a userdata that implements `:wait()` (preferred) and may
also implement `__call()` so it can be awaited via `a()`.
- `async.await(awaitable)` and `async.select(list)` operate on this protocol.
- `Task` and `Channel` are awaitables via `Task:wait()` and `Channel:wait()`.

This contract exists to keep concurrency composable: you can pass awaitables
into `async.select` without "eagerly awaiting" them first.

### Tasks

#### `async.spawn(fn, ...) -> Task`

Spawns a concurrent Lua task that runs `fn(...)`.

```lua
local t = async.spawn(function(x)
  return x + 1, "ok"
end, 41)
```

#### Lifetime and cancellation semantics

Tasks are **structured by default**: if the `Task` handle becomes unreachable
and is garbage-collected (or dropped by losing scope), the underlying task may
be aborted.

If you intentionally want a "fire-and-forget" task, call `t:detach()` to prevent
abort-on-drop. Prefer structured tasks unless you have a clear reason to detach.

#### `Task:wait() -> ...`

Waits for the task to finish and returns the function's return values.

```lua
local n, s = t:wait()
print(n, s) -- 42  ok
```

This makes `Task` compatible with the awaitables contract and with `async.select`.

Errors:

- Raises `"task already joined"` if `wait()` is called more than once.
- Raises `"cancelled"` if the underlying task was aborted (for example, because
the handle was dropped without `detach()`).

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

### Channels

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

- `Channel:wait()` returns `nil, "closed"` only after the sender is closed
**and** the queue is drained.
- If your channel can carry `nil`, always check `err` to distinguish a `nil`
message from closure.

#### `Channel:try_recv() -> value | nil, err`

Sync. Attempts to receive without waiting.

- Returns the value on success.
- Returns `nil, "empty"` if no value is available.
- Returns `nil, "closed"` if the channel is disconnected.

Note: if another task is currently blocked in `wait()`, `try_recv()` may return
`nil, "busy"` (implementation detail). Treat both `"empty"` and `"busy"` as
retryable states.

#### `Channel:close() -> true`

Closes the **sender** side of the channel.

Important: `close()` does **not** discard queued items. Receivers can continue
calling `wait()` until the channel is fully drained and then observe `nil, "closed"`.

### Selecting across awaitables

#### `async.select(list) -> idx, ...`

Races multiple awaitables concurrently and returns the first one that completes.
`list` must be an array-like table of **userdata awaitables**. For convenience,
Ward also accepts:

- `Task` userdata (waited via `:wait()`)
- `Channel` userdata (waited via `:wait()`)

The return value is:

- `idx` - the **1-based** index into `list` that completed first
- followed by that awaitable's return values

Note: `async.select` cancels the non-winning internal *waiters*
(the race participants), but it does not automatically cancel arbitrary
underlying work unless that work is itself cancellation-aware. For
example, if you race a long-running task against a timeout, the task
will continue running unless you explicitly cancel it (or allow it to
be aborted via structured drop/GC behavior).

Example: race a task against a timeout

```lua
local async = require("ward.async")
local time = require("ward.time")

local t = async.spawn(function()
  time.sleep("200ms"):wait()
  return "task"
end)

local idx, v = async.select({ t, time.sleep("50ms") })
print("winner", idx, v)
```

### `async.await(awaitable) -> ...`

Awaits a single awaitable userdata using the awaitables contract
(`:wait()` preferred, or `__call()`).

This is mostly a convenience wrapper for readability and for writing
higher-level helpers.

```lua
local async = require("ward.async")
local time = require("ward.time")

async.await(time.sleep("100ms"))
```

### Examples

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

### Streaming example: watch a long-running command and react to new output

This pattern is common in bash/sh scripting
(e.g., `tail -f ... | while read ...; do ...; done`).
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
