local process = require("ward.process")
local env = require("ward.env")
local fs = require("ward.fs")

-- Notes:
-- - By default, this example uses *non-interactive* mode (-n).
-- - If you want password prompting, do a single pre-authentication call that
--   inherits stdio (sudo -v or doas true), then keep using -n afterwards.
-- - Captured output (Cmd:output) cannot reliably prompt for passwords.

local function pick_priv_tool()
	if env.is_in_path("doas") then
		return "doas"
	end
	if env.is_in_path("sudo") then
		return "sudo"
	end
	return nil
end

local function with_middleware(mw, body)
	process.push_middleware(mw)
	local ok, err = pcall(body)
	process.pop_middleware()
	if not ok then
		error(err)
	end
end

local function privileged_middleware(opts)
	opts = opts or {}
	local tool = assert(opts.tool, "opts.tool is required")
	local non_interactive = opts.non_interactive ~= false

	return function(spec)
		local argv = spec.argv or {}
		if #argv == 0 then
			return nil
		end

		-- Avoid double-wrapping.
		if argv[1] == tool then
			return nil
		end

		local out = { tool }

		-- Both sudo and doas accept -n for non-interactive.
		if non_interactive then
			table.insert(out, "-n")
		end

		-- Disambiguate options vs program.
		table.insert(out, "--")

		for i = 1, #argv do
			table.insert(out, argv[i])
		end

		spec.argv = out
		return nil
	end
end

local tool = pick_priv_tool()
if not tool then
	error("Neither sudo nor doas found in PATH")
end

with_middleware(privileged_middleware({ tool = tool, non_interactive = false }), function()
	-- Any process invocation inside this scope is now automatically wrapped.
	local user_id = process.cmd("id", "-un"):output()
	print("You are user:", require("ward.helpers.string").trim(user_id.stdout))

	-- Example of a privileged read.
	if fs.is_exists("/etc/shadow") then
		local r = process.cmd("sh", "-lc", "test -r /etc/shadow && echo readable || echo not_readable"):output()
		print("/etc/passwd is:", r.stdout)
	end
end)
