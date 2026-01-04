# ward.module

`ward.module` is Ward's external Lua module manager. It downloads third-party
Lua code into Ward's data directory and makes it available to the current
process via `require("externals.<name>")`. Downloads are stored in a
content-addressed layout so multiple versions can coexist and be reused across runs.

## Import

```lua
local module = require("ward.module")
```

## Where modules live

Ward keeps external modules under an "externals" directory in the user data home:

- Base directory: `${XDG_DATA_HOME}/ward/externals`
  - If `XDG_DATA_HOME` is not set: `~/.local/share/ward/externals`
- Content-addressed store: `${base}/.store/<id>`

The store id is derived from the normalized source URL and the selected
revision (for git sources), so changing `tag`/`branch`/`rev` produces a
different `<id>`.

Ward maintains a per-process binding map so that once you bind `externals.<name>`
to an id, future `require("externals.<name>")` calls in that same run resolve to
the same revision. Rebinding is rejected unless you pass `force=true`, in which
case Ward also clears the cached `require` entry so the next `require` reloads.

## Naming rules

If you don't specify a `name`, it is derived from the URL:

- Basename is taken from the URL path; common suffixes are removed
(`.git`, `.lua`, `.zip`, `.tar.gz`, `.tgz`), and query/fragment parts are ignored.
- The resulting name is canonicalized:
  - lowercase
  - non-alphanumeric separators collapse to `_`
  - names starting with a digit are prefixed with `_`

Submodules are supported:

```lua
require("externals.<name>.<submodule>")
```

(Each segment must be alphanumeric or `_`.)

## API

### module.dir() -> string

Returns the absolute path to the externals base directory.

```lua
local module = require("ward.module")
print(module.dir())
```

### module.git(url, opts?) -> { ok, name, require, path, store_path, id }

Fetches a git repository, checks out a selected revision, and binds it as
`externals.<name>` for this process.

**Arguments**

- `url` (string): git repository URL (HTTPS/SSH, etc.)
- `opts` (table, optional):
  - `name` (string): override derived module name / binding name
  - Exactly one selector:
    - `rev` (string): commit hash
    - `branch` (string): branch name
    - `tag` (string): tag name
    - If none provided, defaults to "head"
  - `force` (boolean, default `false`): allow rebinding `externals.<name>` to a
  different id (also clears cached `require`)
  - `depth` (integer): shallow clone depth
  - `recursive` (boolean): fetch submodules
  - `timeout` (number, seconds): overall fetch timeout
  - `max_bytes` (integer): abort if transfer exceeds this size
  - `filter_blobs` (boolean): request partial clone with blob filtering (when supported)

**Return fields**

- `ok` (boolean): whether fetch/checkout succeeded
- `name` (string): canonicalized name
- `require` (string): require target (e.g. `externals.my_lib`)
- `path` / `store_path` (string): checkout directory path
- `id` (string): content-addressed id for this URL+selector

**Example**

```lua
local module = require("ward.module")

local result = module.git("https://github.com/org/repo.git", {
  name = "my_lib",
  tag = "v1.2.3",
  depth = 1,
  recursive = true,
})

assert(result.ok, "git fetch failed")

-- Require a submodule from the checked out repo
local api = require(result.require .. ".api")
print("loaded", result.require, "from", result.path, "id", result.id)
```

### module.url(url, opts?) -> { ok, name, require, path, store\_path, id }

Downloads a single Lua file and binds it as `externals.<name>`. The downloaded
content is stored as `init.lua` inside the store directory for its id.

**Arguments**

- `url` (string): HTTP(S) URL to a `.lua` file (or any endpoint returning Lua content)
- `opts` (table, optional):
  - `name` (string): override derived module name
  - `insecure` (boolean, default `false`): allow invalid TLS certificates
  - `follow_redirects` (boolean, default `true`)
  - `retries` (integer, default `5`, minimum `1`)
  - `retry_delay` (integer, milliseconds, default `2000`)
  - `force` (boolean, default `false`): allow rebinding `externals.<name>` to a
  new id
  - `timeout` (number, seconds): overall request timeout
  - `max_bytes` (integer): abort if download exceeds this size
  - `method` (string): HTTP method (default `GET`)
  - `headers` (table\<string, string>): additional request headers

**Return fields** Same shape as `module.git`.

**Example**

```lua
local module = require("ward.module")

local result = module.url("https://example.com/plugin.lua", {
  headers = { Authorization = "Bearer TOKEN" },
  method = "POST",
  retry_delay = 500,
})

assert(result.ok, "download failed")

local plugin = require(result.require)
print("plugin loaded:", plugin.version and plugin.version() or "<no version()>")
```

## Notes and best practices

- Prefer pinning git modules with `tag` or `rev` to get reproducible behavior.
- Use `force=true` only when you intentionally want to rebind a name within a
single run.
- If you need multiple versions at once, bind them under different names
(e.g. `my_lib_v1`, `my_lib_v2`) rather than forcing rebinding.
