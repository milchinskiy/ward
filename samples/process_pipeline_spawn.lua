local p = require("ward.process")

-- Demonstrates spawning a pipeline and consuming its stdout as a stream.
--
-- Try:
--   ward run samples/process_pipeline_spawn.lua

local child = (p.cmd("sh", "-lc", "printf 'a\\nb\\nc\\n' && sleep 0.2") | p.cmd("grep", "b"))
  :spawn({ stdout = true })

local out = assert(child:stdout_lines())

while true do
  local line, err = out:wait()
  if not line then break end
  print("line:", line)
end

child:wait()
