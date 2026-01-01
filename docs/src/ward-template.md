- [`minijinja.render(template, context, opts?) -> string`](#minijinjarendertemplate-context-opts---string)
- [`minijinja.render_async(template, context, opts?) -> string`](#minijinjarender_asynctemplate-context-opts---string)
- [`minijinja.render_file(path, context, opts?) -> string`](#minijinjarender_filepath-context-opts---string)
- [`minijinja.render_file_async(path, context, opts?) -> string`](#minijinjarender_file_asyncpath-context-opts---string)
- [Options](#175-options)

MiniJinja is a fast, Rust-native Jinja2-like template engine.

```lua
local tpl = require("ward.template.minijinja")
```

### `minijinja.render(template, context, opts?) -> string`

Render a template string using the given `context` (Lua table).

```lua
local out = tpl.render("Hello {{ user.name }}!", {
  user = { name = "Ward" }
})
print(out) -- Hello Ward!
```

### `minijinja.render_async(template, context, opts?) -> string`

Same as `render`, but runs the render on a blocking thread so it will not
block the async runtime.

```lua
local out = tpl.render_async("{{ n }}", { n = 42 })
print(out)
```

### `minijinja.render_file(path, context, opts?) -> string`

Read a file from `path` and render its contents as a template.

```lua
local out = tpl.render_file("./hello.tmpl", { name = "world" })
print(out)
```

### `minijinja.render_file_async(path, context, opts?) -> string`

Same as `render_file`, but runs on a blocking thread.

### Options

All functions accept an optional `opts` table:

- `undefined`: one of `"strict"`, `"lenient"`, `"chainable"` (default: `"strict"`)
- `trim_blocks`: boolean (default: `false`)
- `lstrip_blocks`: boolean (default: `false`)
- `keep_trailing_newline`: boolean (default: `false`)
- `auto_escape`: boolean (default: `false`)
- `loader`: table configuring `{% include %}` / `{% import %}` resolution
  - `paths`: array of strings; additional search paths for templates

Example:

```lua
local out = tpl.render_file("./templates/main.j2", {
  title = "Hello",
  items = { "a", "b", "c" },
}, {
  undefined = "strict",
  trim_blocks = true,
  lstrip_blocks = true,
  loader = {
    paths = { "./templates" },
  },
})
print(out)
```
