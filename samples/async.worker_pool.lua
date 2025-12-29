local async = require("ward.async")
local process = require("ward.process")
local time = require("ward.time")

-- Worker-pool sample:
--   * A bounded jobs channel provides backpressure.
--   * N workers consume jobs concurrently.
--   * A results channel fans results back in to the main coroutine.
--
-- This demonstrates practical "concurrency" in Ward (async tasks) without any extra primitives
-- beyond async.spawn + async.channel.

local WORKERS = 4
local JOBS = 12

-- Small capacity to demonstrate backpressure (producer will await if full).
local jobs = async.channel({ capacity = 4 })
local results = async.channel({ capacity = JOBS })

local tasks = {}

for wid = 1, WORKERS do
	tasks[wid] = async.spawn(function()
		while true do
			local job, err = jobs:wait()
			if not job then
				-- jobs channel closed/drained
				break
			end

			local ok, payload = pcall(function()
				-- Simulate work variance.
				time.sleep(string.format("%dms", 50 * (wid % 3 + 1))):wait()

				-- Do some real I/O (async under the hood): run a trivial command.
				local r = process
					.cmd("sh", "-lc", "echo worker=" .. tostring(wid) .. " job=" .. tostring(job) .. "; uname -s")
					:output()

				return {
					job = job,
					worker = wid,
					ok = r.ok,
					code = r.code,
					out = r.stdout,
				}
			end)

			if ok then
				-- If results is full, this will await, providing backpressure to workers.
				results:send(payload)
			else
				-- Never let a worker silently die; report error via results.
				results:send({
					job = job,
					worker = wid,
					ok = false,
					err = tostring(payload),
				})
			end
		end
	end)
end

-- Producer runs concurrently so results are drained while jobs are still being queued.
local producer = async.spawn(function()
	for j = 1, JOBS do
		jobs:send(j)
	end
	jobs:close()
end)

-- Consumer: collect exactly JOBS results.
-- (We collect results while workers run, to avoid blocking workers on a full results channel.)
for _ = 1, JOBS do
	local msg, err = results:wait()
	if not msg then
		error("results channel closed early: " .. tostring(err))
	end

	if msg.ok then
		print(
			string.format(
				"done job=%d worker=%d ok=%s code=%s out=%s",
				msg.job,
				msg.worker,
				tostring(msg.ok),
				tostring(msg.code),
				tostring(msg.out)
			)
		)
	else
		print(
			string.format("failed job=%s worker=%s err=%s", tostring(msg.job), tostring(msg.worker), tostring(msg.err))
		)
	end
end

producer:wait()

-- Wait for workers to finish (jobs is closed; they should exit promptly).
for i = 1, WORKERS do
	tasks[i]:wait()
end

results:close()
print("all done")
