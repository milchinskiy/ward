# About

- [Project goals](#project-goals)
- [Problems Ward solves](#problems-ward-solves-and-why-not-just-bashsh)
- [What Ward is not](#what-ward-is-not)
- [Core design principles you can rely on](#core-design-principles-you-can-rely-on)
- [A quick taste: "shell replacement" style](#a-quick-taste-shell-replacement-style)
- [Importing modules](#importing-modules)

Ward is a Lua runtime for writing **system scripts** that are pleasant and safe like a real language,
but still feel as direct as bash/sh for day-to-day automation.

Think of Ward as:

- **A scripting runtime** (you write Lua)
- **A standard library for systems work** (fs, env, process, net, term, time, etc.)
- **A "shell-like" UX** for pipelines, exit codes, and stdout/stderr — without shell footguns

## Project goals

Ward is designed to help you write scripts that are:

- **Readable**: explicit modules, explicit options, explicit error handling
- **Composable**: small primitives that chain well (process pipelines, fs + fetch, etc.)
- **Predictable**: fewer implicit behaviors than shells; structured results instead of string parsing
- **Practical**: excellent at orchestrating external tools (git, tar, curl-like flows, build tools)
- **Cross-platform by default**: consistent APIs across Linux/macOS/Windows where feasible (in progress)

## Problems Ward solves (and why not just bash/sh)

Shell scripting is powerful, but it tends to degrade quickly as complexity grows:

- Error handling is subtle (`set -e`, pipelines, subshells, traps)
- Quoting/escaping is fragile, especially with spaces/newlines and binary data
- Data is usually "strings all the way down" (hard to validate, hard to refactor)
- Concurrency and streaming require non-trivial patterns
- Portability across `sh`/`bash`/`dash` and different OSes is painful

Ward addresses these by offering shell-like primitives with language-level structure:

- **Explicit process execution** (`ward.process`) with structured results (`ok`, exit code, stdout/stderr)
- **Filesystem utilities** (`ward.fs`) with consistent return conventions for mutating operations
- **Environment overlay** (`ward.env`) so scripts can modify env safely without global side effects
- **Network fetch + HTTP** (`ward.net`) that composes with `ward.fs`
- **Terminal UI** (`ward.term`) for prompts, progress, and ANSI utilities

## What Ward is not

Ward is intentionally not trying to become:

- A full interactive shell replacement (job control, shell syntax, glob semantics everywhere)
- A configuration-management/orchestration framework (inventory/targeting is out of scope for now)
- A CPU-parallel compute runtime for Lua (Ward focuses on I/O overlap and orchestration)

Ward is a tool for writing robust scripts that run external programs and manipulate files reliably.

## Core design principles you can rely on

**1) Explicit modules**
Ward APIs live under `ward.*` and are imported via `require`. This makes scripts easy to audit and refactor.

**2) Structured boundaries**
Where shells typically return strings and exit codes, Ward usually returns either:

- a **structured userdata** (e.g., `CmdResult`)
- or a **result table** like `{ ok, err }` for mutating filesystem operations

**3) Script-friendly errors**
Errors are meant to be understandable and printable. Many APIs are designed so you can adopt a
"set -e" style by asserting success early and failing fast.

**4) Async-capable runtime (without forcing you into a new programming model)**
Ward runs on an async-capable runtime. Many operations internally use async I/O, but most APIs are
still called in a straightforward way. Some operations return *awaitable* objects when the operation
must be explicitly awaited (interactive input, some timers, streaming reads, etc.).

A full explanation of awaitables, tasks, channels, and `select` is in the async section (see `ward.async`).

## A quick taste: "shell replacement" style

A Ward script typically reads like "do X, assert it worked, then proceed":

```lua
local process = require("ward.process")

process.cmd("git", "rev-parse", "--is-inside-work-tree")
  :output()
  :assert_ok("not a git repo")

local r = (process.cmd("printf", "hello\nworld\n") | process.cmd("wc", "-l")):output()
r:assert_ok()
print("lines:", r.stdout)
```

## Importing modules

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

> Tip: Prefer importing the specific submodules you use; it keeps scripts explicit and makes the
> dependency surface obvious.
