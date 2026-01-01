local p = require("ward.process")

-- Demonstrates interactive stdin (writing to a long-running process).
--
-- Try:
--   ward run samples/process_interactive.lua

-- cat echoes stdin back to stdout.
local child = p.cmd("cat"):spawn({ stdin = true, stdout = true })

local stdin = assert(child:stdin())
local out = assert(child:stdout_lines())

-- Write two lines and close stdin.
stdin:writeln("hello")
stdin:writeln("world")
stdin:close()

-- Read echoed lines.
while true do
	local line, err = out:wait()
	if not line then
		break
	end
	print("got:", line)
end

child:wait()
