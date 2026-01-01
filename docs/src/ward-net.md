- [`ward.net.http` - HTTP request primitives](#wardnethttp---http-request-primitives)
- [`ward.net.fetch` - fetch into a file/dir](#wardnetfetch---fetch-into-a-filedir)
- [`ward.module` - external module manager](#wardmodule---external-module-manager)
  - [`module.dir() -> string`](#moduledir---string)
  - [`module.git(url, opts?) -> { ok, name, require, path, store_path, id }`](#modulegiturl-opts---ok-name-require-path-store_path-id-)
  - [`module.url(url, opts?) -> { ok, name, require, path, store_path, id }`](#moduleurlurl-opts---ok-name-require-path-store_path-id-)

```lua
local net   = require("ward.net")
local http  = require("ward.net.http")
local fetch = require("ward.net.fetch")
```

`ward.net` groups network-related helpers. Today it exposes two submodules:

- `ward.net.http` - in-process HTTP requests via `reqwest`
- `ward.net.fetch` - higher-level "fetch into a file/dir" helpers

### `ward.net.http` - HTTP request primitives

#### Functions

All functions below are **async** (implemented with `create_async_function`).
Call them normally and receive a `HttpResponse` userdata.

- `http.get(url, opts?) -> HttpResponse`
- `http.delete(url, opts?) -> HttpResponse`
- `http.options(url, opts?) -> HttpResponse`
- `http.post(url, opts?) -> HttpResponse`
- `http.put(url, opts?) -> HttpResponse`

#### Options (`opts` table)

`opts` is optional. When omitted or not a table, defaults are applied.

- `query` (table) - query parameters.
  - Keys are strings.
  - Values must be `string`, `number`, `integer`, or `boolean` (they are
  converted to strings).
- `headers` (table) - header map: `string -> string`.
- `timeout` (number) - request timeout in **seconds** (float accepted). Must be
positive and finite.
- `follow_redirects` (boolean, default `true`) - when enabled, redirects are
followed (limited to 10).
- `allow_error` (boolean, default `false`) -
  - `false`: non-2xx responses raise a runtime error.
  - `true`: non-2xx responses are returned as `HttpResponse`.

Body options (used by `post` and `put`):

- `json` (any) - serializable Lua value encoded as JSON.
- `form` (table) - form fields `string -> string`.

If both `json` and `form` are present, JSON takes precedence.

#### `HttpResponse` userdata

Returned by `http.*` functions.

Fields (also available via methods):

- `resp.status` (integer) - HTTP status code.
- `resp.headers` (table) - header map `string -> string`.
  - Note: duplicate header names will be overwritten in the table (last one wins).
- `resp.body` (string|nil) - response body decoded as text.

Methods:

- `resp:is_ok() -> boolean` - true for 2xx.
- `resp:status() -> integer`
- `resp:headers() -> table`
- `resp:body() -> string|nil`

Examples:

```lua
local http = require("ward.net.http")

-- Basic GET
local r = http.get("https://example.com", { follow_redirects = true })
print(r.status)
print(r:is_ok())

-- Query + headers
local r2 = http.get("https://httpbin.org/get", {
  query = { q = "ward", page = 1, debug = true },
  headers = { ["User-Agent"] = "ward" },
  timeout = 10,
  allow_error = true,
})
print(r2.status)
print(r2:body())

-- POST JSON
local r3 = http.post("https://httpbin.org/post", {
  json = { hello = "world", n = 1 },
  headers = { ["Content-Type"] = "application/json" },
})
assert(r3:is_ok())
```

### `ward.net.fetch` - fetch into a file/dir

`fetch` is for "download/checkout into a path" workflows that compose well with `ward.fs`.

#### `fetch.url(url, opts?) -> FetchResponse`

Downloads the response body as bytes into a file (streaming), then returns metadata.

Options (`opts` table):

- `method` (string, default `"GET"`) - one of: `GET`, `POST`, `PUT`, `PATCH`,
`DELETE`, `HEAD`, `OPTIONS`.
- `headers` (table) - header map `string -> string`.
- `timeout` (number) - request timeout in **seconds**.
- `follow_redirects` (boolean, default `true`) - redirects are followed
- `insecure` (boolean, default `false`) - allow invalid TLS certificates.
(limited to 10).
- `into` (string|nil) - destination file path.
  - If omitted, Ward creates a unique file path under the OS temp directory.
- `max_bytes` (integer|number|nil) - maximum allowed response size.
  - Values `<= 0` disable the limit.
  - If the limit is exceeded, Ward removes the partial file and returns
`ok=false` with `status=413` and `path=nil`.

`FetchResponse` userdata fields/methods:

- `resp.ok` / `resp:is_ok() -> boolean`
- `resp.status` / `resp:status() -> integer` - HTTP status code (or `413`
if `max_bytes` exceeded).
- `resp.path` / `resp:path() -> string|nil` - destination path.
- `resp.size` / `resp:size() -> integer` - bytes written.

Example:

```lua
local fetch = require("ward.net.fetch")
local fs    = require("ward.fs")

local r = fetch.url("https://example.com/file.tar.gz", {
  into = "./downloads/file.tar.gz",
  max_bytes = 50 * 1024 * 1024,
})

if not r.ok then
  error("fetch failed: status=" .. tostring(r.status))
end

print("saved to", r.path, "bytes", r.size)
assert(fs.is_file(r.path))
```

#### `fetch.git(url, opts?) -> FetchResponse`

Clones a Git repository into a directory (using the external `git` command),
then optionally checks out a revision.

Notes:

- Requires `git` to be installed and discoverable in `PATH`
(it is relies on a system `git` binary).
- `git` stdout/stderr are suppressed; use `ok`/`status` to handle errors.

Options (`opts` table):

- `into` (string|nil) - destination directory.
  - If omitted, Ward creates a unique directory under the OS temp directory.
- `depth` (integer|nil) - shallow clone depth (must be > 0). Defaults to `1`.
- `filter_blobs` (boolean, default `true`) - when true, uses `--filter=blob:none`.
- `branch` (string|nil) - clone a specific branch.
- `tag` (string|nil) - clone a specific tag (used as `--branch <tag>`).
  - If both `branch` and `tag` are set, `branch` takes precedence.
- `recursive` (boolean, default `false`) - when true, uses `--recurse-submodules`.
- `rev` (string|nil) - if set, runs `git checkout <rev>` after cloning.
- `timeout` (number|nil) - command timeout in **seconds**.
- `max_bytes` (integer|number|nil) - maximum allowed on-disk size for the cloned
directory.
  - If exceeded, Ward removes the directory and returns `ok=false` with
`status=413` and `path=nil`.

On success, `FetchResponse.status` is `0` and `ok=true`. On failure, `status` is
the `git` exit code.

Example:

```lua
local fetch = require("ward.net.fetch")

local r = fetch.git("https://github.com/user/repo.git", {
  into = "./vendor/repo",
  depth = 1,
  rev = "v1.2.3",
  recursive = false,
  timeout = 120,
})

if not r.ok then
  error("git fetch failed: exit=" .. tostring(r.status))
end

print("checked out into", r.path, "bytes", r.size)
```

### `ward.module` - external module manager

`ward.module` downloads third-party Lua modules into Ward's data directory and
exposes them via `require("externals.<name>")` for the lifetime of the
process. Downloads are content-addressed so multiple revisions can coexist and
be reused across runs.

```lua
local module = require("ward.module")
```

#### Where modules live

- Base directory: `${XDG_DATA_HOME}/ward/externals` (or
  `~/.local/share/ward/externals` if `XDG_DATA_HOME` is unset).
- Content-addressed store: `${base}/.store/<id>`, where `<id>` is
  `sha256(normalized_url + "\n" + selector)`.
- A per-process binding map ensures `require("externals.<name>")` resolves to
  the revision selected earlier in the same run. Rebinding to a different id is
  rejected unless `force=true`, in which case the next `require` reloads the
  module.

#### Naming rules

- Default name comes from the URL basename (extensions, `.git`, `.lua`, `.zip`,
  `.tar.gz`, `.tgz`, and query/fragment removed).
- Names are canonicalized to lowercase with non-alphanumeric separators folded
  into `_`. Leading digits are prefixed with `_`.
- `require("externals.<name>.<submodule>")` is supported; segments must be
  alphanumeric or `_`.

#### `module.dir() -> string`

Returns the absolute path to the externals base directory.

```lua
print(module.dir())
```

#### `module.git(url, opts?) -> { ok, name, require, path, store_path, id }`

Clone a Git repository into the externals store and bind it for `require`.

Options (all optional):

- `name` (string) - override derived name and `externals.<name>` binding.
- Exactly one of `rev`, `branch`, or `tag` selects a revision; default is
  `head`. Different selectors produce different store ids.
- `force` (bool, default `false`) - allow rebinding an already-bound name to a
  new id (also clears cached `require`d module).
- `depth` (integer) - shallow clone depth (passed to `ward.net.fetch.git`).
- `recursive` (bool) - fetch submodules (`git --recurse-submodules`).
- `timeout` (number, seconds) - overall fetch timeout.
- `max_bytes` (integer) - abort if transfer exceeds this many bytes.
- `filter_blobs` (bool) - request partial clone with blob filtering when
  supported by the remote.

Return table:

- `ok` (bool) - whether the checkout succeeded.
- `name` (string) - canonicalized name.
- `require` (string) - require target (`externals.<name>`).
- `path` / `store_path` (string) - final checkout directory.
- `id` (string) - content-addressed id.

Example (pin to a tag and require a submodule):

```lua
local module = require("ward.module")
local json = require("ward.convert.json")

local result = module.git("https://github.com/org/repo.git", {
  name = "my_lib",
  tag = "v1.2.3",
  depth = 1,
  recursive = true,
})

assert(result.ok, "git fetch failed")
local api = require(result.require .. ".api")
print(json.encode({ id = result.id, path = result.path, ok = result.ok }))
```

#### `module.url(url, opts?) -> { ok, name, require, path, store_path, id }`

Download a single Lua file and bind it as `externals.<name>`. The URL content is
stored as `init.lua` inside its store directory.

Options (all optional):

- `name` (string) - override derived name.
- `insecure` (bool, default `false`) - allow invalid TLS certificates.
- `follow_redirects` (bool, default `true`).
- `retries` (integer, default `5`, minimum `1`).
- `retry_delay` (milliseconds, default `2000`) - delay between retries.
- `force` (bool, default `false`) - rebind an existing name to the downloaded id.
- `timeout` (number, seconds) - overall request timeout.
- `max_bytes` (integer) - abort if download exceeds this size.
- `method` (string) - HTTP method (default `GET`).
- `headers` (table<string, string>) - additional request headers.

Return table fields match `module.git`.

Example (with custom headers and POST body):

```lua
local module = require("ward.module")

local result = module.url("https://example.com/plugin.lua", {
  headers = { Authorization = "Bearer TOKEN" },
  method = "POST",
  retry_delay = 500,
})

if not result.ok then error("download failed") end
local plugin = require(result.require)
print(plugin.version())
```
