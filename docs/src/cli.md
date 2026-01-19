# CLI

Ward ships a single executable, `ward`, which provides a small set of commands
for running Lua-based system scripts.

This page documents the CLI behavior (command syntax, argument forwarding, exit
codes, sandbox options) and includes usage examples.

## Common concepts

### Argument forwarding (`arg`)

All Ward commands that execute Lua (`run`, `eval`, `repl`) forward trailing CLI
arguments to the Lua program and expose them via the standard Lua global `arg`.

- `arg[0]` identifies the entry being executed:
  - `run`: the script path
  - `eval`: `(eval)`
  - `repl`: `(repl)`
- `arg[1]..arg[n]` are the forwarded arguments.

It is recommended to use `--` to terminate Ward's CLI parsing when you are
forwarding arbitrary flags.

### Exit codes

- If the Lua program finishes normally, Ward exits with code `0`.
- If Lua raises an unhandled error, Ward prints the error and exits with a
non-zero code.
- If a script calls `require("ward.process").exit(code)`, Ward exits with
exactly that code.
- In `repl`, `Ctrl-C` exits with code `130`.

### Sandbox options

`run`, `eval`, and `repl` share the same sandbox limit switches:

- `-m, --memory-limit <bytes>`: memory cap for the Lua VM (bytes)
- `-i, --instruction-limit <n>`: approximate instruction cap (enforced via VM
hook every ~1024 instructions)
- `-t, --threads <n>`: Tokio worker thread count (also used for the Lua async
pool)
- `-T, --timeout <duration>`: wall-clock timeout (supports seconds or duration
strings like `500ms`, `2s`, `1m`)

For details on instruction-limit behavior, see the notes in [Conventions](conventions.md).

### Logging

Ward's runtime logging level can be configured through:

- `WARD_LOG=trace|debug|info|warn|error|fatal`

## `ward run`

Run a Lua file.

### Synopsis

```text
ward run [OPTIONS] FILE [--] [ARGS...]
```

### Examples

Run a script and forward positional args:

```bash
ward run ./script.lua -- foo bar
```

Forward flags without ambiguity:

```bash
ward run ./script.lua -- --flag --other-flag=value
```

Shebang mode (Ward strips a leading `#!` line):

```lua
#!/usr/bin/env -S ward run
print("hello from ward")
```

## `ward eval`

Evaluate a Lua chunk provided either via `--expr` or via `stdin`.

### Synopsis

```text
ward eval [OPTIONS] [-e|--expr LUA] [--] [ARGS...]
```

If `--expr` is omitted, `ward eval` reads code from `stdin` unless
`--no-stdin` is provided.

### Options

- `-e, --expr <lua>`: Lua code to evaluate.
- `--no-stdin`: do not read code from stdin when `--expr` is not provided.

All sandbox options described above are supported.

### Examples

Evaluate a one-liner:

```bash
ward eval -e 'print("hello")'
```

Use forwarded arguments:

```bash
ward eval -e 'print(arg[0]); print(arg[1])' -- foo
# prints:
# (eval)
# foo
```

Read code from stdin:

```bash
echo 'print("from stdin")' | ward eval
```

Explicitly refuse stdin (useful for CI scripts that should fail fast):

```bash
ward eval --no-stdin
# error: no code provided (use --expr or pipe into stdin)
```

## `ward repl`

Start an interactive Lua REPL inside the Ward runtime.

The REPL keeps a single Lua state for the entire session, so definitions
persist between inputs.

### Synopsis

```text
ward repl [OPTIONS] [--] [ARGS...]
```

### Options

- `--no-prompt`: disable prompts and banner (useful when piping input).

All sandbox options described above are supported.

### Exiting the REPL

You can exit in any of the following ways:

- Press `Ctrl-D` (EOF)
- Type one of: `exit`, `quit`, `:q`, `:quit`
- Call `require("ward.process").exit(code)`

`Ctrl-C` exits with code `130`.

### Multi-line input

The REPL supports multi-line chunks: if the current input is syntactically
incomplete, Ward switches the prompt to `>>` and keeps reading until the
chunk is complete.

### Convenience: `=expr` shorthand

If you enter a single line that starts with `=`, Ward rewrites it as `print(<expr>)`.

Example:

```text
> =1+2
3
```

### Examples

Start a REPL and inspect the platform:

```text
$ ward repl
> local p = require("ward.host.platform")
> print(p.os())
linux
```

Use forwarded args in a REPL session:

```text
$ ward repl -- hello
> print(arg[0])
(repl)
> print(arg[1])
hello
```

Use the REPL as a non-interactive evaluator (use `--no-prompt` to keep output clean):

```bash
printf 'print("hi")\n' | ward repl --no-prompt
```
