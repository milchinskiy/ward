local p = require("ward.process")

-- basic
p.cmd("echo", "hello"):run():assert_ok()

-- capture
local r = p.cmd("git", "rev-parse", "HEAD"):output()
print(r.stdout)

-- pipeline with |
local r = (p.cmd("ps", "aux") | p.cmd("grep", "nginx") | p.cmd("head", "-n", "5")):output()
print(r.stdout)

-- pipeline with :pipe()
local r = p.cmd("ps", "aux"):pipe(p.cmd("grep", "nginx")):pipe(p.cmd("head", "-n", "5")):output()
print(r.stdout)

-- merge stderr into pipeline (like 2>&1 | ...)
local r = (p.cmd("sh", "-lc", "echo out; echo err 1>&2"):stderr_to_stdout(true) | p.cmd("grep", "err")):output()
print(r.stdout)
