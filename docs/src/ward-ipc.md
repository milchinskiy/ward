- [Connecting and listening](#connecting-and-listening)
- [Streams](#streams)
- [Examples](#examples)

Fast local IPC using Unix domain sockets. Available on Unix platforms (Linux,
macOS). This module integrates with Ward's awaitable I/O model.

### Connecting and listening

- `unix.connect(path, opts?) -> UnixStream`
  - Connects to `path` and returns a stream. Options are currently reserved;
    unknown fields raise errors to avoid silent misconfiguration.

- `unix.listen(path, opts?) -> UnixListener`
  - Default behavior removes stale socket paths: if a prior server crashed and
    left the path behind, Ward attempts a connection; a refused/not-found
    result removes the path, while a successful connection errors with
    "address in use".
  - Options:
    - `backlog`: positive integer (default 128).
    - `mode`: permission bits (use `0o660` or `tonumber("660", 8)`).
    - `owner` / `group`: numeric uid/gid to chown after bind.
    - `unlink` (default `true`): when `false`, existing paths abort instead of
      being removed.
    - `unlink_on_close` (default `true`): unlink the path when the listener is
      closed/dropped (only for paths created by Ward).
    - `mkdir`: create parent directories before binding.

### Streams

`UnixStream` methods (all return awaitables yielding `{value}|nil, err`):

- `read(n?)` (default 16384 bytes), `read_exact(n)`
- `write(bytes)` (returns bytes written), `write_all(bytes)`
- `shutdown()` (close write half), `close()` (drop both halves)
- Awaitable helpers: `wait(n?)` and calling the stream itself delegates to
  `read(n?)`.
- Field: `stream.closed` (boolean snapshot).

`UnixListener`:

- `accept() -> UnixStream|nil, err`
- `close() -> boolean` (unlinks if owned and configured).

### Examples

Echo server + client:

```lua
local async = require("ward.async")
local unix = require("ward.ipc.unix")
local json = require("ward.convert.json")

local socket = "/tmp/ward-echo.sock"
local listener = assert(unix.listen(socket, {
  backlog = 8,
  mode = tonumber("660", 8),
  unlink_on_close = true,
}))

async.spawn(function()
  while true do
    local stream, err = listener:accept()
    if not stream then return print("accept failed: " .. tostring(err)) end

    async.spawn(function()
      while true do
        local data, rerr = stream:read(1024)
        if not data then return stream:close() end
        stream:write_all(data):wait()
      end
    end)
  end
end):detach()

local client = assert(unix.connect(socket):wait())
assert(client:write_all("ping"):wait())
local resp = assert(client:read(4):wait())

print(json.encode({ response = resp }))
```

See `samples/ipc_unix_echo.lua` for a complete sample including mkdir, stale
cleanup, and permission settings.
