### Common

- `nil|string` means the function returns either `nil` or a Lua string.
- "bytes string" means a Lua string whose contents are raw bytes (binary-safe).
- Paths are typically accepted as strings (and in most ward.fs APIs also as
ward.fs.path objects).
- Most option tables are optional; when omitted, defaults apply.

---

### CLI sandbox limits (`ward run`)

`ward run` executes a Lua file under a configurable sandbox. The most commonly
used switches are:

- `--memory-limit BYTES` - maximum Lua memory usage
- `--instruction-limit N` - approximate instruction budget (see notes below)
- `--timeout SECONDS` - wall-clock timeout (also accepts duration strings like
`500ms`, `2s`, `1m`)
- `--threads N` - Tokio worker threads

Notes:

- **Instruction limiting is intentionally coarse.** Ward installs a Lua VM
hook every 1024 instructions (or less if the configured limit is smaller),
so a script may exceed the configured limit by up to 1023 instructions.
- The instruction hook does not execute while awaiting Rust async operations
(e.g., I/O). Long-running I/O does not consume the instruction budget.
- It helps prevent runaway scripts (memory/time/instructions). This is not a
secure sandbox for untrusted code.
