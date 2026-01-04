# ward.time

- [Wall clock](#wall-clock)
- [Parsing](#parsing)
- [Construction](#construction)
- [Durations](#durations)
- [Monotonic time](#monotonic-time)
- [Timers (return awaitables)](#timers-return-awaitables)
- [Blocking sleep](#blocking-sleep)

```lua
local time = require("ward.time")
```

### Wall clock

- `time.now() -> TimePoint`
- `time.now_table() -> table` - returns a table with timestamp components

### Parsing

- `time.parse_rfc3339(s) -> TimePoint|nil`
- `time.parse_rfc2822(s) -> TimePoint|nil`
- `time.parse(s) -> TimePoint|nil` - best-effort parser

### Construction

- `time.from_timestamp(seconds, nanos?) -> TimePoint`
- `time.utc(y, m, d, hh?, mm?, ss?, nanos?) -> TimePoint`

### Durations

#### `time.duration(x) -> Duration`

Accepts:

- number: whole seconds
- string: human readable duration (e.g. `500ms`, `1.5s`, `2h`)
- Duration userdata

Examples:

```lua
local d1 = time.duration(2)
local d2 = time.duration("250ms")
local d3 = time.duration("2h")
```

### Monotonic time

- `time.instant_now() -> InstantPoint`

### Timers (return awaitables)

These return userdata you must call `()` or `:wait()`.

- `time.sleep(duration) -> SleepAwaitable`
- `time.after(duration, callback?) -> AfterAwaitable`
- `time.interval(duration) -> IntervalTimer`
- `time.timeout(awaitable, duration) -> TimeoutAwaitable`

Examples:

```lua
-- sleep
time.sleep("200ms"):wait()

-- after
local v = time.after("1s", function() return "done" end):wait()
print(v)

-- interval
local it = time.interval("1s")
for _ = 1, 3 do
  print("tick", it())
end

-- timeout
local a = time.sleep("5s")
local ok, err = pcall(function()
  time.timeout(a, "200ms"):wait()
end)
print(ok, err)
```

### Blocking sleep

- `time.sleep_blocking(duration) -> true`
