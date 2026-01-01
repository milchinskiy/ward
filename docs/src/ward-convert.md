- [`ward.convert.json`](#wardconvertjson)
- [`ward.convert.yaml`](#wardconvertyaml)
- [`ward.convert.toml`](#wardconverttoml)
- [`ward.convert.ini`](#wardconvertini)

```lua
local convert = require("ward.convert")
local json = require("ward.convert.json")
local yaml = require("ward.convert.yaml")
local toml = require("ward.convert.toml")
local ini  = require("ward.convert.ini")
```

Common patterns:

- `encode(value) -> string`
- `decode(string) -> value`
- `encode_async(value) -> string`
- `decode_async(string) -> value`

Example (JSON):

```lua
local json = require("ward.convert.json")
local s = json.encode({ a = 1, b = { true, false } })
local t = json.decode(s)
```

### `ward.convert.json`

Functions:

- `json.encode(value, opts?) -> string`
- `json.decode(text) -> value`
- `json.encode_async(value, opts?) -> string` (runs on a blocking thread)
- `json.decode_async(text) -> value` (runs on a blocking thread)

`opts` for `encode`/`encode_async`:

- `pretty` (boolean, default `false`) - pretty-print.
- `indent` (integer, default `2`) - spaces per indent level (must be > 0).

### `ward.convert.yaml`

Functions:

- `yaml.encode(value) -> string`
- `yaml.decode(text) -> value`
- `yaml.encode_async(value) -> string` (blocking thread)
- `yaml.decode_async(text) -> value` (blocking thread)

### `ward.convert.toml`

Functions:

- `toml.encode(value) -> string`
- `toml.decode(text) -> value`
- `toml.encode_async(value) -> string` (blocking thread)
- `toml.decode_async(text) -> value` (blocking thread)

### `ward.convert.ini`

Functions:

- `ini.encode(table) -> string`
- `ini.decode(text) -> table`
- `ini.encode_async(table) -> string` (blocking thread)
- `ini.decode_async(text) -> table` (blocking thread)

`ini.encode` expects a table shaped like:

```lua
{
  [""] = { root_key = "value" },       -- optional default section
  section = { key1 = "v1", flag = true }
}
```

All values are stringified; booleans/numbers are accepted and converted to
strings.
