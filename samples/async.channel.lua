local async = require("ward.async")
local log = require("ward.log")
local p = require("ward.process")
local time = require("ward.time")

-- Demonstrates:
--   * spawning concurrent Lua tasks
--   * communicating results via a bounded channel
--
-- NOTE: Ward runs Lua in an async-capable mode, so calling async Rust methods
-- (process, time, net, etc.) looks synchronous from Lua.

local workers = 5
local ch = async.channel({ capacity = 16 })
local tasks = {}

for i = 1, workers do
	tasks[i] = async.spawn(function()
		local ok, res = pcall(function()
			-- Simulate staggered workloads.
			time.sleep(0.1 * i):wait()

			-- Run a subprocess (async under the hood).
			local r = p.cmd("sh", "-lc", "echo worker=" .. tostring(i) .. "; uname -s"):output()
			return {
				i = i,
				ok = r.ok,
				code = r.code,
				out = r.stdout,
			}
		end)

		if ok then
			ch:send(res)
		else
			-- Avoid silent hangs: report failure via channel.
			ch:send({ i = i, ok = false, err = tostring(res) })
		end
	end)
end

for _ = 1, workers do
	local msg, err = ch:wait()
	if not msg then
		error("channel closed: " .. tostring(err))
	end

	if msg.ok then
		log.info(
			string.format(
				"result from worker %d: ok=%s code=%s out=%s",
				msg.i,
				tostring(msg.ok),
				tostring(msg.code),
				tostring(msg.out)
			)
		)
	else
		log.error(string.format("worker %d failed: %s", msg.i, tostring(msg.err)))
	end
end

for i = 1, workers do
	tasks[i]:wait()
end

ch:close()
