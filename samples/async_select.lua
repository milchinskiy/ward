-- Demonstrates async.select() across a Channel and a Timer in one loop.
--
-- Behavior:
--   - A producer task sends a few messages to a channel, then closes it.
--   - The main loop waits on:
--       async.select({ ch, timer })
--     so it can process messages immediately, while also getting timer ticks
--     (useful for timeouts, heartbeats, debouncing, etc.).

local async = require("ward.async")
local time = require("ward.time")

local ch = async.channel({ capacity = 16 })
local tick_every = "250ms"
local timer = time.sleep(tick_every) -- one-shot timer; we re-arm after it fires

-- Producer: emit some messages on its own cadence, then close the channel.
local producer = async.spawn(function()
	for i = 1, 5 do
		time.sleep("400ms"):wait() -- awaitable sleep; `()` calls the awaitable
		ch:send({ n = i, msg = "hello" })
	end
	ch:close()
end)

print("waiting: select({channel, timer})")

local tick = 0
while true do
	-- async.select returns: idx, ...winner_return_values
	local idx, a, b = async.select({ ch, timer })

	if idx == 1 then
		-- channel:wait() returns: msg OR nil, "closed"
		local msg, err = a, b
		if not msg then
			print("channel done:", err)
			break
		end
		print("wait:", msg.n, msg.msg)
	else
		-- timer fired (sleep returns nothing); re-arm it
		tick = tick + 1
		print("tick:", tick)
		timer = time.sleep(tick_every)
	end
end

producer:wait()
print("done")
