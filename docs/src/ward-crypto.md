# ward.crypto

```lua
local crypto = require("ward.crypto")
```

Byte-string functions (Lua strings are binary-safe):

- `crypto.sha256(bytes) -> string` (hex)
- `crypto.sha1(bytes) -> string` (hex)
- `crypto.md5(bytes) -> string` (hex)

File functions (async, streamed):

- `crypto.sha256_file(path) -> string` (hex)
- `crypto.sha1_file(path) -> string` (hex)
- `crypto.md5_file(path) -> string` (hex)

Examples:

```lua
local crypto = require("ward.crypto")

local digest = crypto.sha256("abc")
print("sha256(abc) =", digest)

local file_digest = crypto.sha256_file("Cargo.toml")
print("sha256(Cargo.toml) =", file_digest)
```

