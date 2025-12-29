local p = require("ward.process")

-- Demonstrates stdout byte streaming (chunk reads).
--
-- Try:
--   ward run samples/process_bytes.lua

-- Emit three bytes: 0x41 'A', 0x00 NUL, 0x42 'B'
local child = p.cmd("sh", "-lc", "printf 'A\\0B'"):spawn({ stdout = true })
local bytes = assert(child:stdout_bytes())

-- ByteStream is awaitable; :wait(n) reads up to n bytes.
local chunk, err = bytes:wait(3)
assert(chunk, err)

print("len:", #chunk)
print("bytes:", string.byte(chunk, 1, #chunk))

child:wait()
