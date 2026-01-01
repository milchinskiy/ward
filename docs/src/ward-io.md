- [`io.read_all(opts?) -> bytes string`](#ioread_allopts---bytes-string)
- [`io.read_line() -> byts string|nil`](#ioread_line---byts-stringnil)
- [`io.read_lines() -> function`](#ioread_lines---function)
- [Output](#output)

```lua
local io = require("ward.io")
```

Ward serializes reads/writes with internal mutexes so concurrent operations
do not interleave unpredictably.

### `io.read_all(opts?) -> bytes string`

Reads all remaining stdin into a string.

Optional `opts`:

- `max_bytes` (number|integer) - if provided, fails when stdin exceeds this limit.

```lua
local s = io.read_all()

-- hard cap (1 MiB)
local s2 = io.read_all({ max_bytes = 1024 * 1024 })
```

### `io.read_line() -> byts string|nil`

Reads one line from stdin.

- Returns `nil` on EOF.

```lua
local line = io.read_line()
if line == nil then return end
print("got:", line)
```

### `io.read_lines() -> function`

Returns an iterator-like function. Each call reads one line from stdin and
returns `bytes string|nil` (nil on EOF).

```lua
local next_line = io.read_lines()
while true do
  local line = next_line()
  if line == nil then break end
  print(line)
end
```

### Output

- `io.write_stdout(data) -> true`
- `io.write_stderr(data) -> true`
- `io.flush_stdout() -> nil`
- `io.flush_stderr() -> nil`

```lua
io.write_stdout("hello")
io.write_stderr("warn\n")
io.flush_stdout()
```
