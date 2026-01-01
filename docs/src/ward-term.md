- [Input (returns awaitables)](#input-returns-awaitables)
- [Printing](#printing)
- [Screen control and tty](#screen-control-and-tty)
- [`term.ansi` submodule](#termansi-submodule)
- [`term.progress(args?) -> Progress`](#termprogressargs---progress)

```lua
local term = require("ward.term")
```

### Input (returns awaitables)

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
local name = term.prompt({ question = "Name", default = "guest" }):wait()
print("hello", name)
```

#### `term.confirm{ question, default? }`

Accepted answers: `y/yes` and `n/no` (case-insensitive). Empty input returns
the `default` if provided.

```lua
local ok = term.confirm({ question = "Continue?", default = false }):wait()
if not ok then return end
```

#### `term.password{ prompt, trim? }`

Reads a line with no echo (TTY).

```lua
local secret = term.password({ prompt = "Password:" }):wait()
```

#### `term.choose{ question, options, default? }`

`options` is an array-like table.

```lua
local choice = term.choose({
  question = "Pick one",
  options = { "dev", "staging", "prod" },
  default = "dev",
}):wait()
print(choice)
```

### Printing

- `term.print(...) -> true`
- `term.println(...) -> true`
- `term.eprint(...) -> true`
- `term.eprintln(...) -> true`

### Screen control and tty

- `term.clear() -> true`
- `term.isatty(stream?) -> boolean` - `stream` can be `"stdout"` or `"stderr"`

### `term.ansi` submodule

`term.ansi` is a table of ANSI escape-code strings you can concatenate into output.

Common style fields:

- `ansi.reset`
- `ansi.bold`, `ansi.dim`, `ansi.italic`, `ansi.underline`, `ansi.blink`,
`ansi.reverse`, `ansi.hidden`, `ansi.strike`

Clear / cursor fields:

- `ansi.clear_line`, `ansi.clear_screen`, `ansi.home`

Colors (foreground):

- `ansi.black`, `ansi.red`, `ansi.green`, `ansi.yellow`, `ansi.blue`,
`ansi.magenta`, `ansi.cyan`, `ansi.white`, `ansi.default`
- `ansi.bright_black`, `ansi.bright_red`, `ansi.bright_green`,
`ansi.bright_yellow`, `ansi.bright_blue`, `ansi.bright_magenta`,
`ansi.bright_cyan`, `ansi.bright_white`

Colors (background):

- `ansi.bg_black`, `ansi.bg_red`, `ansi.bg_green`, `ansi.bg_yellow`,
`ansi.bg_blue`, `ansi.bg_magenta`, `ansi.bg_cyan`, `ansi.bg_white`, `ansi.bg_default`
- `ansi.bg_bright_black`, `ansi.bg_bright_red`, `ansi.bg_bright_green`,
`ansi.bg_bright_yellow`, `ansi.bg_bright_blue`, `ansi.bg_bright_magenta`,
`ansi.bg_bright_cyan`, `ansi.bg_bright_white`

Example:

```lua
local ansi = term.ansi
term.println(ansi.bold .. ansi.green .. "OK" .. ansi.reset)
```

### `term.progress(args?) -> Progress`

Create a progress renderer for TTY output.

Constructor args (table):

- `total` (integer|nil)
- `message` (string|nil)
- `stream` (`"stdout"|"stderr"`, default `"stderr"`)

Progress methods (getter/setter style):

- `p:tick(delta?) -> nil` - increment by `delta` (default 1)
- `p:value(v?) -> integer|nil` - get current when called without args;
set when `v` provided
- `p:total(t?) -> integer|nil` - get total when called without args;
set when `t` provided
- `p:message(s?) -> string|nil` - get message when called without args;
set when `s` provided
- `p:finish(final_msg?) -> true` - render final line + newline (TTY only)

Example:

```lua
local p = term.progress({ total = 10, message = "Working" })
for _ = 1, 10 do
  time.sleep("150ms"):wait()
  p:tick(1)
end
p:finish("Done")
```
