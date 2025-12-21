local env = require("ward.env")
local tpl = require("ward.template.minijinja")
local c = require("ward.crypto")

local templ = [[
#!{{ binbash }}

echo "Hello, {{ name|capitalize }}!"
echo "{{ crypt|escape }}"
]]

print(
	tpl.render(
		templ,
		{ binbash = env.which("bash"), name = "ward", crypt = c.sha256("hello") },
		{ lstrip_blocks = true, trim_blocks = true }
	)
)
