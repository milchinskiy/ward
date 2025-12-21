local env = require("ward.env")
local tpl = require("ward.template.minijinja")

print(tpl.render(
	[[
#!{{ binbash }}

echo "Hello, {{ name|capitalize }}!"
]],
	{ binbash = env.which("bash"), name = "ward" },
	{ lstrip_blocks = true, trim_blocks = true }
))
