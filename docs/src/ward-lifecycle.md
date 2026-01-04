# ward.lifecycle

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

Functions:

- `lifecycle.on(signal, fn) -> id` - register a handler for a signal name/number
  (`"HUP"`, `"INT"`, `"QUIT"`, `"USR1"`, `"USR2"`, `"PIPE"`, `"TERM"`, or a numeric code).
  Returns a handler id.
- `lifecycle.on_shutdown(fn) -> id` - register a shutdown callback (LIFO order).
- `lifecycle.off(id) -> boolean` - remove a previously registered handler.
- `lifecycle.request(code?) -> ()` - request shutdown; `code` is an optional exit code.
- `lifecycle.requested() -> boolean` - whether shutdown has been requested.
- `lifecycle.code() -> integer|nil` - shutdown exit code (if known).

Notes:

- Ctrl-C / SIGINT requests shutdown by default.
- Shutdown callbacks run once in LIFO order; failures are best-effort and do not
  prevent later callbacks.

