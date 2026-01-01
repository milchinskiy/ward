```lua
local log = require("ward.log")
log.info("hello", "world")
log.trace(...)
log.debug(...)
log.warn(...)
log.error(...)
log.fatal(...)
```

Ward log is intentionally minimal; use it for script-friendly logs.

