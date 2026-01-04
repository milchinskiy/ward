# ward.helpers

- [`helpers.number`](#helpersnumber)
- [`helpers.string`](#helpersstring)
- [`helpers.table`](#helperstable)
- [`helpers.retry`](#helpersretry)

```lua
local helpers = require("ward.helpers")
local retry   = require("ward.helpers.retry")
local n = require("ward.helpers.number")
local s = require("ward.helpers.string")
local t = require("ward.helpers.table")
```

### `helpers.number`

Numeric predicates and aggregates. All functions return errors if arguments have
the wrong type.

Functions:

- `is_integer(n) -> boolean` - `true` when `n` is finite and has no fractional component.
- `is_float(n) -> boolean` - `true` for finite, non-integer numbers.
- `is_number(v) -> boolean` - `true` for Lua integers or floats.
- `is_nan(v) -> boolean` - `true` when `v` is a float NaN.
- `is_infinity(v) -> boolean` - `true` when `v` is `+/-inf`.
- `is_finite(v) -> boolean` - `true` for integers and finite floats.
- `round(n, precision) -> number` - rounds `n` to `precision` decimal places.
- `ceil(n) -> number` - ceiling.
- `floor(n) -> number` - floor.
- `abs(n) -> number` - absolute value.
- `clamp(n, min, max) -> number` - clamps `n` into `[min, max]`.
- `sign(n) -> integer` - returns `1`, `0`, or `-1`.
- `random(min, max) -> number` - uniform random between `min` and `max`
(inclusive for integers).
- `avg(...) -> number` - average of variadic arguments (at least one required).
- `min(...) -> number` - minimum of variadic arguments (at least one required).
- `max(...) -> number` - maximum of variadic arguments (at least one required).
- `sum(...) -> number` - sum of variadic arguments (at least one required).

### `helpers.string`

String utilities and regex helpers. Patterns use Rust regex syntax.

Functions:

- `trim(s) -> string` - trim both ends.
- `ltrim(s) -> string` - trim start.
- `rtrim(s) -> string` - trim end.
- `contains(s, substr) -> boolean` - substring test.
- `replace(s, substr, repl) -> string` - replace first occurrence.
- `replace_all(s, substr, repl) -> string` - replace all occurrences.
- `split(s, sep) -> table` - splits into an array table.
- `join(sep, ...) -> string` - joins variadic strings with `sep`.
- `to_lower(s) -> string` - lowercase.
- `to_upper(s) -> string` - uppercase.
- `to_title(s) -> string` - title-case words.
- `to_snake(s) -> string` - snake_case.
- `to_camel(s) -> string` - camelCase.
- `to_kebab(s) -> string` - kebab-case.
- `to_pascal(s) -> string` - PascalCase.
- `to_slug(s) -> string` - slug (alias of kebab-case).
- `match(s, pattern) -> table` - first regex match captures as array (empty
table if none).
- `match_all(s, pattern) -> table` - list of capture arrays for all matches.
- `match_replace(s, pattern, repl) -> string` - replace first regex match.
- `match_replace_all(s, pattern, repl) -> string` - replace all regex matches.

### `helpers.table`

Functional-style table helpers. Unless noted, sequence order is preserved and
non-sequence keys are ignored. Functions that expect an array treat numeric
keys from `1..n` as the sequence.

Functions:

- `is_empty(tbl) -> boolean` - `true` when table has no key/value pairs.
- `contains(tbl, value) -> boolean` - `true` if any value equals `value`.
- `concat(tbl, ...) -> table` - concatenates array tables.
- `merge(tbl, ...) -> table` - shallow merge (later values overwrite).
- `deep_merge(tbl, ...) -> table` - recursive merge for nested tables.
- `map(tbl, fn) -> table` - applies `fn(value, key)`; returns table with same keys.
- `filter(tbl, fn) -> table` - keeps entries where `fn(value, key)` returns `true`.
- `reduce(tbl, fn, acc) -> any` - folds with `fn(acc, value, key)`.
- `each(tbl, fn) -> table` - calls `fn(value, key)` for side effects; returns
original table.
- `find(tbl, fn) -> any|nil` - first value where `fn(value, key)` returns
`true`, else `nil`.
- `findall(tbl, fn) -> table` - array of values where predicate is `true`.
- `sort(tbl, fn) -> table` - returns sorted copy using comparator
`fn(a, b) -> boolean` (`true` when `a < b`).
- `reverse(tbl) -> table` - reversed array copy.
- `shuffle(tbl) -> table` - shuffled array copy.
- `flatten(tbl) -> table` - recursively flattens nested array tables.
- `uniq(tbl) -> table` - removes duplicates (first occurrence wins).
- `uniq_by(tbl, fn) -> table` - uniqueness by computed key `fn(value, key)`.
- `count(tbl, fn) -> integer` - number of entries where predicate is `true`.
- `keys(tbl) -> table` - array of keys.
- `values(tbl) -> table` - array of values.
- `push(tbl, value) -> nil` - append to end.
- `append(tbl, value) -> nil` - alias of `push`.
- `pop(tbl) -> any|nil` - remove and return last element (or `nil`).
- `shift(tbl) -> any|nil` - remove and return first element (or `nil`).
- `prepend(tbl, value) -> nil` - insert at start.
- `join(tbl, sep) -> string` - stringifies sequence values with separator.

### `helpers.retry`

helpers.retry implements an async retry loop for functions that may
intermittently fail.

#### `retry.run(fn, opts?) -> any`

Calls fn() and returns its result. If fn() errors, Ward retries until success
or the attempt limit is reached.

Options (opts table):

- attempts (integer, default 3) - total attempts (minimum 1)
- delay (duration, default `100ms`) - base delay between retries; accepts
strings like `50ms`, `1.5s`, `2m`
- max_delay (duration|nil) - optional cap on the delay; accepts the same
duration strings
- backoff (number, default 2.0) - multiplier applied to the delay after each
failed attempt (minimum 1.0)
- jitter (boolean, default false) - randomize delay to reduce thundering herd
- jitter_ratio (number, default 0.2) - maximum relative jitter, clamped to 0..1

Example:

```lua
local retry = require("ward.helpers.retry")
local net = require("ward.net.http")

local res = retry.run(function()
    ...
end, { attempts = 5, delay = "200ms", backoff = 2.0, jitter = true })

print("ok:", res.status)
```
