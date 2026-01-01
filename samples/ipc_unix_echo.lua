local async = require("ward.async")
local unix = require("ward.ipc.unix")
local json = require("ward.convert.json")
local socket_path = string.format("/tmp/ward-echo-%d.sock", math.random(1, 1e9))

-- Ensure the parent directory exists and clean up stale paths on start.
local listener, err = unix.listen(socket_path, {
	backlog = 8,
	unlink_on_close = true,
	mkdir = true,
	mode = tonumber("660", 8),
})
assert(listener, err)

-- Server task: accept one client and echo once.
local server_task = async.spawn(function()
	local stream, accept_err = listener:accept()
	if not stream then
		return nil, accept_err
	end

	local data, read_err = stream:read(1024)
	if not data then
		return nil, read_err
	end

	local ok, write_err = stream:write_all(data)
	stream:close()
	if not ok then
		return nil, write_err
	end

	return data
end)

-- Client task.
local client_task = async.spawn(function()
	local client = assert(unix.connect(socket_path))
	assert(client:write_all("hello"))
	local resp = assert(client:read(5))
	client:close()
	return resp
end)

local client_resp = client_task:wait()
listener:close()
local echoed, echo_err = server_task:wait()
assert(echoed, echo_err)

print(json.encode({ socket = socket_path, response = client_resp }))
