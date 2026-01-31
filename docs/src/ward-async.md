# `ward.async` - tasks and channels

- [Overview](#overview)
- [Awaitables contract (important)](#awaitables-contract-important)
- [Tasks](#tasks)
- [Channels](#channels)
- [Selecting across awaitables](#selecting-across-awaitables)
- [`async.await(awaitable) -> ...`](#asyncawaitawaitable---)

```lua
local async = require("ward.async")
```

## Overview

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

## Awaitables contract (important)

Ward uses a single user-facing await protocol:

- An **awaitable** is a userdata that implements `:wait()` (preferred) and may
also implement `__call()` so it can be awaited via `a()`.
- `async.await(awaitable)` and `async.select(list)` operate on this protocol.
- `Task` and `Channel` are awaitables via `Task:wait()` and `Channel:wait()`.

This contract exists to keep concurrency composable: you can pass awaitables
into `async.select` without "eagerly awaiting" them first.

## Tasks

### `async.spawn(fn, ...) -> Task`

Spawns a concurrent Lua task that runs `fn(...)`.

```lua
local t = async.spawn(function(x)
  return x + 1, "ok"
end, 41)
```

### Lifetime and cancellation semantics

Tasks are **structured by default**: if the `Task` handle becomes unreachable
and is garbage-collected (or dropped by losing scope), the underlying task may
be aborted.

If you intentionally want a "fire-and-forget" task, call `t:detach()` to prevent
abort-on-drop. Prefer structured tasks unless you have a clear reason to detach.

### `Task:wait() -> ...`

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

### `Task:cancel() -> boolean`

Requests task cancellation.

- Returns `true` if cancellation was requested.
- Returns `false` if the task is already finished (or already cancelled).

### `Task:done() -> boolean`

Returns `true` when the task has finished.

### `Task:detach() -> boolean`

Detaches the task from structured cancellation-on-drop/GC.

- Returns `true` after switching the task into detached mode.
- After detaching, dropping the `Task` handle will **not** abort the underlying task.

## Channels

### `async.channel(opts?) -> Channel`

Creates a bounded channel.

Accepted `opts`:

- `nil` (defaults apply)
- a number (capacity)
- a table: `{ capacity = N }`

If omitted, capacity defaults to **64**.

```lua
local ch = async.channel({ capacity = 16 })
```

### `Channel:send(value) -> true | nil, err`

Async. Sends a value into the channel.

- Returns `true` on success.
- Returns `nil, "closed"` if the channel is closed.

### `Channel:try_send(value) -> true | nil, err`

Sync. Attempts to send without waiting.

- Returns `true` on success.
- Returns `nil, "full"` if the buffer is full.
- Returns `nil, "closed"` if the channel is closed.

### `Channel:wait() -> value | nil, err`

Async. Receives a value from the channel.

- Returns the value on success.
- Returns `nil, "closed"` after the sender is closed **and** the queue is drained.

This makes `Channel` compatible with the awaitables contract and with `async.select`.

Notes:

- `Channel:wait()` returns `nil, "closed"` only after the sender is closed
**and** the queue is drained.
- If your channel can carry `nil`, always check `err` to distinguish a `nil`
message from closure.

### `Channel:try_recv() -> value | nil, err`

Sync. Attempts to receive without waiting.

- Returns the value on success.
- Returns `nil, "empty"` if no value is available.
- Returns `nil, "closed"` if the channel is disconnected.

Note: if another task is currently blocked in `wait()`, `try_recv()` may return
`nil, "busy"` (implementation detail). Treat both `"empty"` and `"busy"` as
retryable states.

### `Channel:close() -> true`

Closes the **sender** side of the channel.

Important: `close()` does **not** discard queued items. Receivers can continue
calling `wait()` until the channel is fully drained and then observe `nil, "closed"`.

## Selecting across awaitables

### `async.select(list) -> idx, ...`

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

## `async.await(awaitable) -> ...`

Awaits a single awaitable userdata using the awaitables contract
(`:wait()` preferred, or `__call()`).

This is mostly a convenience wrapper for readability and for writing
higher-level helpers.

```lua
local async = require("ward.async")
local time = require("ward.time")

async.await(time.sleep("100ms"))
```

