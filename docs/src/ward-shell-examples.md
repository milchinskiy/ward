- ["set -e" style: assert on failures](#set--e-style-assert-on-failures)
- [Download to temp dir and process](#download-to-temp-dir-and-process)

### "set -e" style: assert on failures

```lua
local process = require("ward.process")

process.cmd("git", "rev-parse", "--is-inside-work-tree")
  :output()
  :assert_ok("not a git repo")
```

### Download to temp dir and process

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
  time.sleep("200ms"):wait()
  prog:tick(1)
end
prog:finish("Done")
```
