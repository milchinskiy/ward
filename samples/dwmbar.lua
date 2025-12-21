-- Minimal dwm bar updater:
--   - multiple interval loops (different cadence)
--   - shared state updated through a single channel
--   - renderer updates xsetroot with a single string

local async = require("ward.async")
local time = require("ward.time")
local process = require("ward.process")
local log = require("ward.log")

-- Where updates from all loops go:
local updates = async.channel({ capacity = 128 })

-- Latest values by key:
local state = {
	cpu = "cpu:?",
	mem = "mem:?",
	bat = "bat:?",
	date = "date:?",
}

local function trim(s)
	return (s:gsub("%s+$", ""))
end

local function sh_output(cmd)
	-- Uses /bin/sh; keep commands fast.
	local r = process.cmd("sh", "-lc", cmd):output()
	if not r.ok then
		return nil, ("cmd failed (code=%s): %s"):format(tostring(r.code), cmd)
	end
	return trim(r.stdout or "")
end

local function publish(key, value)
	-- Ignore backpressure failures (shouldn't happen with capacity 128 unless something is wrong).
	local ok, err = updates:try_send({ key = key, value = value })
	if not ok and err ~= "full" then
		-- closed or unexpected; nothing to do for a bar script
	end
end

local function start_interval(name, seconds, fn)
	return async.spawn(function()
		-- Push first value immediately so the bar fills quickly.
		local ok, val = pcall(fn)
		if ok and val then
			publish(name, val)
		else
			publish(name, name .. ":err")
		end

		local ticker = time.interval(seconds)
		while true do
			ticker:wait()
			local ok2, val2 = pcall(fn)
			if ok2 and val2 then
				publish(name, val2)
			else
				publish(name, name .. ":err")
			end
		end
	end)
end

-- Render the complete bar string from latest state.
local function render()
	-- Order is up to you:
	return ("%s | %s | %s | %s"):format(state.cpu, state.mem, state.bat, state.date)
end

local result_str = ""
local function set_root_name(s)
	log.trace("set_root_name: " .. s)
	if s == result_str then
		return
	end
	result_str = s
	print(result_str)
	-- If you want to debug, replace with: print(s)
	-- process.cmd("xsetroot", "-name", s):run()
end

-- --- Interval loops (customize commands) ---

-- CPU: quick and dirty loadavg (portable)
local cpu_task = start_interval("cpu", 1.5, function()
	local out = sh_output('awk \'{print $1" "$2" "$3}\' /proc/loadavg')
	return "cpu:" .. (out or "?")
end)

-- Memory: MemAvailable
local mem_task = start_interval("mem", 2.0, function()
	local out = sh_output([[awk '/MemAvailable/ {printf "%.1fG", $2/1024/1024}' /proc/meminfo]])
	return "mem:" .. (out or "?")
end)

-- Battery: adjust BAT0 if needed; falls back gracefully on desktops
local bat_task = start_interval("bat", 2.0, function()
	local out = sh_output([[
    if [ -d /sys/class/power_supply/BAT0 ]; then
      cap=$(cat /sys/class/power_supply/BAT0/capacity 2>/dev/null || echo "?")
      st=$(cat /sys/class/power_supply/BAT0/status 2>/dev/null || echo "?")
      echo "${cap}% ${st}"
    else
      echo "AC"
    fi
  ]])
	return "bat:" .. (out or "?")
end)

-- Date/time
local date_task = start_interval("date", 1, function()
	local out = sh_output([[date '+%Y-%m-%d %H:%M:%S']])
	return out or "date:?"
end)

require("ward.lifecycle").on_shutdown(function()
    log.info("shutting down")
    cpu_task:cancel()
    mem_task:cancel()
    bat_task:cancel()
    date_task:cancel()
end)

-- --- Renderer loop ---
-- Update bar whenever we receive any update.
-- (No select needed: we just block on updates:recv().)
while true do
	local msg, err = updates:recv()
	if not msg then
		-- Should not happen unless channel is closed
		break
	end

	state[msg.key] = msg.value
	set_root_name(render())
end

-- If you ever add shutdown handling, you can cancel/join tasks here.
-- cpu_task:cancel(); mem_task:cancel(); bat_task:cancel(); date_task:cancel()
