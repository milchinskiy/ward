-- examples/progress.lua
local term = require("ward.term")
local time = require("ward.time")

-- Convenience: sleep wrapper (adapt if your time API differs)
local function sleep_ms(ms)
  -- if you have time.sleep(ms) as awaitable:
  -- return time.sleep(ms)()
  -- otherwise adapt to your actual time module.
  return time.sleep(ms / 1000):wait()
end

local ansi = term.ansi

-- 1) Spinner-style progress (unknown total)
do
  local p = term.progress({
    message = "Probing environment",
    spinner = true,        -- if your API supports it; harmless otherwise
  })

  for i = 1, 30 do
    p:tick()               -- spinner tick
    if i == 10 then p:message("Probing filesystem") end
    if i == 20 then p:message("Probing network") end
    sleep_ms(50)
  end

  p:finish(ansi.green .. "OK" .. ansi.reset)
end

print()

-- 2) Determinate progress (known total)
do
  local total = 100
  local p = term.progress({
    total = total,
    message = "Downloading artifacts",
  })

  for i = 1, total do
    -- emulate work
    sleep_ms(15)

    -- update progress and optional message
    p:set(i)
    if i == 1 then
      p:message("Connecting")
    elseif i == 20 then
      p:message("Receiving data")
    elseif i == 80 then
      p:message("Finalizing")
    end
  end

  p:finish(ansi.bright_green .. "DONE" .. ansi.reset)
end

print()

-- 3) Using term.confirm + progress (typical shell script flow)
do
  local ok = term.confirm({ question = "Run a slow task?", default = false }):wait()
  if not ok then
    print("Canceled.")
    return
  end

  local p = term.progress({ total = 5, message = "Slow task" })
  for i = 1, 5 do
    p:set(i)
    p:message(("Step %d/5"):format(i))
    sleep_ms(250)
  end
  p:finish(ansi.cyan .. "Complete" .. ansi.reset)
end
