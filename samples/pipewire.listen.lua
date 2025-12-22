local p = require("ward.process")
local str = require("ward.helpers.string")

-- Demonstrates streaming stdout line-by-line from a long-running process.
--
-- Goal:
--   Watch `pw-mon -oap` output (stream) and, when a *new* event line appears
--   that contains `PipeWire:Interface:Device`, run:
--     wpctl get-volume @DEFAULT_AUDIO_SINK@
--   and print its value.

local NEEDLE = "PipeWire:Interface:Device"

local child = p.cmd("pw-mon", "-oap"):spawn({ stdout = true })
local out, oerr = child:stdout_lines()
assert(out, oerr)

print("watching pw-mon ...")

while true do
	local line, lerr = out:wait()
	if not line then
        require("ward.log").error(lerr)
		break
	end

	if str.contains(line, NEEDLE) then
		local r = p.cmd("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"):output()
		if r.ok then
			print(str.trim(r.stdout or ""))
		end
	end
end
