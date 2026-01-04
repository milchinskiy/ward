# ward.fs

- [Existence and type checks](#existence-and-type-checks)
- [Path utilities](#path-utilities)
  - [`fs.path` - pure path manipulation (Path userdata)](#fspath---pure-path-manipulation-path-userdata)
- [Directory listing and globbing](#directory-listing-and-globbing)
- [Directories and removal](#directories-and-removal)
- [Permissions and links](#permissions-and-links)
- [Timestamps](#timestamps)
- [File IO](#file-io)
- [Copy and move](#copy-and-move)
- [Temporary directories](#temporary-directories)

```lua
local fs = require("ward.fs")
```

### 4.1 Existence and type checks

- `fs.is_exists(path) -> boolean`
- `fs.is_dir(path) -> boolean`
- `fs.is_file(path) -> boolean`
- `fs.is_link(path) -> boolean`
- `fs.is_symlink(path) -> boolean`
- `fs.is_block_device(path) -> boolean` (Unix)
- `fs.is_char_device(path) -> boolean` (Unix)
- `fs.is_fifo(path) -> boolean` (Unix)
- `fs.is_socket(path) -> boolean` (Unix)
- `fs.is_executable(path) -> boolean`
- `fs.is_readable(path) -> boolean`
- `fs.is_writable(path) -> boolean`

Notes:

- For files, `fs.is_readable/fs.is_writable` check whether the file can be
opened for read/write.
- For directories, `fs.is_readable` checks whether the directory can be
listed (`read_dir`), and
  `fs.is_writable` checks whether a temporary file can be created and removed
inside the directory.

Example:

```lua
if fs.is_file("./build.sh") and fs.is_executable("./build.sh") then
  print("runnable")
end
```

### Path utilities

- `fs.readlink(path) -> string|nil`
- `fs.realpath(path) -> string|nil`
- `fs.dirname(path) -> string`
- `fs.basename(path) -> string`
- `fs.join(a, b, ...) -> string`

Example:

```lua
local p = fs.join("build", "out", "app.bin")
print(fs.dirname(p))
print(fs.basename(p))
```

#### fs.path` - pure path manipulation (Path userdata)

`fs.path` provides a `Path` userdata type for manipulating paths **without**
touching the filesystem.
It is useful for building paths safely and passing them to `ward.fs` APIs.

Constructors:

- `fs.path.new(path) -> Path`
- `fs.path.cwd() -> Path`
- `fs.path.join(a, b) -> Path` (both arguments may be strings or `Path`)

Methods on `Path`:

- `Path:is_abs() -> boolean`
- `Path:normalize() -> Path`
- `Path:parts() -> table` (array-like, path components)
- `Path:split() -> (dirname: string, basename: string)`
- `Path:join(segment) -> Path`
- `Path:dirname() -> string`
- `Path:basename() -> string`
- `Path:extname() -> nil|string`
- `Path:stem() -> nil|string`
- `Path:as_string() -> string`

Interoperability:

Most `ward.fs` functions accept either a path string **or** a `fs.path` object.

Example:

```lua
local fs = require("ward.fs")
local path = require("ward.fs.path")

local p = path.new("build/../out/app.bin"):normalize()
assert(fs.mkdir(p:dirname(), { recursive = true, force = true }).ok)
assert(fs.write(p, "hello\n").ok)
print("wrote:", tostring(p))
```

### Directory listing and globbing

#### `fs.list(path, opts?) -> table`

Returns an array-like table of paths.

Options (`opts` table):

- `recursive` (boolean, default `false`)
- `depth` (integer, default `0`) - recursion depth limit; `0` means unlimited
- `dirs` (boolean, default `false`) - include directories
- `files` (boolean, default `false`) - include files
- `regex` (string|nil) - regex filter applied to the full path string

Notes:

- If both `dirs` and `files` are `false` (the default), both directories and
files are included.
- Ordering is OS-dependent (Ward does not currently sort results).

Examples:

```lua
-- list everything
for _, p in ipairs(fs.list(".")) do
  print(p)
end

-- only files, recursive, depth-limited
local files = fs.list("src", { recursive = true, depth = 3, files = true })
```

#### `fs.glob(pattern) -> table`

Returns an array-like table of paths matching a glob pattern.

```lua
for _, p in ipairs(fs.glob("src/**/*.rs")) do
  print(p)
end
```

### Directories and removal

**Result convention for mutating operations:** most functions that change the
filesystem return a table `{ ok, err }` where `ok` is a boolean and `err` is
a string (or `nil` on success). When `ok` is `false`, `err` is intended to be
human-readable and suitable for printing/logging.

#### `fs.mkdir(path, opts?) -> { ok, err }`

Create directory.

Options:

- `recursive` (boolean, default `false`)
- `mode` (number, unix-only; default `0o755`)
- `force` (boolean, default `false`) - treat "already exists" as success

```lua
assert(fs.mkdir("build", { recursive = true, mode = 0o755 }).ok)
```

#### `fs.rm(path, opts?) -> { ok, err }`

Remove file or directory.

Options:

- `recursive` (boolean, default `false`) - required for directories
- `force` (boolean, default `false`) - treat missing-path as success

```lua
assert(fs.rm("build", { recursive = true, force = true }).ok)
```

#### `fs.unlink(path, opts?) -> { ok, err }`

Remove a file (like `rm -f file`).

### Permissions and links

- `fs.chmod(path, mode) -> { ok, err }`
- `fs.chown(path, uid, gid) -> { ok, err }` (Unix)
- `fs.rename(from, to) -> { ok, err }`
- `fs.link(from, to) -> { ok, err }` (hard link)
- `fs.symlink(from, to) -> { ok, err }` (symbolic link)

### Timestamps

#### `fs.touch(path, opts?) -> { ok, err }`

Create file if missing and update timestamps.

Options:

- `recursive` (boolean, default `false`) - create parent directories first

```lua
assert(fs.touch("logs/app.log", { recursive = true }).ok)
```

### File IO

#### `fs.read(path, opts?) -> bytes string`

Reads a file and returns a Lua string containing raw bytes.

Options:

- `mode` (`"text"|"binary"`, default `"text"`)

Notes:

- In `"text"` mode Ward validates UTF-8; the returned Lua string still
contains the original bytes.

```lua
local data = fs.read("README.md")
```

#### `fs.write(path, data, opts?) -> { ok, err }`

Write data to a file.

Options (selected):

- `mode` (`"overwrite"|"append"|"prepend"|"binary"`, default `"overwrite"`)
- `binary` (boolean, default `false`) - convert `data` as bytes

Notes:

- Ward does not automatically create parent directories; combine with
`fs.mkdir(fs.dirname(path), {recursive=true})`.

```lua
assert(fs.write("out.txt", "hello\n").ok)
assert(fs.write("out.txt", "more\n", { mode = "append" }).ok)
```

### Copy and move

- `fs.copy(from, to, opts?) -> { ok, err }`
- `fs.move(from, to, opts?) -> { ok, err }`

Notes:

- `fs.copy` operates on regular files; it does not copy directories.
- `fs.move` uses `rename` when possible. On cross-device moves it falls back
to copy+remove for regular files; moving directories or symlinks across
devices is currently unsupported.

### Temporary directories

#### `fs.tempdir(prefix?) -> string`

Creates a temporary directory and returns its path.

```lua
local dir = fs.tempdir("ward-")
print("tmp:", dir)
```
