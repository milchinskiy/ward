local fetch = require("ward.net.fetch")
local r = fetch.url("https://wttr.in/Tokyo?format=j2", {
    method = "GET",
    headers = {
        ["User-Agent"] = "Ward",
    },
    timeout = 60,
    follow_redirects = true,
    into = "./test-dir",
    max_bytes = 1024 * 1024 * 2, -- 2MB
})

print(r.status)
print(r.ok)
print(r.path)
print(r.size)
