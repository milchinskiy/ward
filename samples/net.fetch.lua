local fetch = require("ward.net.fetch")
local r = fetch.url("https://httpbin.org/get", {
    method = "GET",
    headers = {
        ["User-Agent"] = "Ward",
    },
    timeout = 60,
    follow_redirects = true,
    into = "./test-dir",
    max_bytes = 1024 * 1024 * 2, -- 2MB
})

print(r.status, r:status())
print(r.is_ok, r:ok())
print(r.path, r:path())
print(r.size, r:size())
