# Future considerations

This document closes out the current plan by recording near-term design considerations without prematurely implementing them.

## CLI considerations

### `conch check <fish|bash>`

Optional positional shell argument; already supported.

### `conch init --output <path>`

Likely worthwhile later as a convenience wrapper around stdout redirection.
Not needed for the core compiler semantics.

### Config search conventions

Current behaviour when `--config` is omitted:

- `${XDG_CONFIG_HOME}/conch.{toml,yaml,yml,json}`
- `${XDG_CONFIG_HOME}/conch/config.{toml,yaml,yml,json}`
- each directory in `${XDG_CONFIG_DIRS}` (default `/etc/xdg`) using the same two layouts

Possible future behaviour:

- explicit project config conventions
- richer diagnostics for config-discovery failures

## Config model considerations

### `before` / `after`

Still sufficient for v1.
A future `needs = [...]` relation may be useful if the project wants a semantic distinction between ordering and dependency.

### Source spans

Worth adding later for better diagnostics.
Would help with:

- pointing to exact conflicting keys
- richer parse/validation errors
- explain output linked back to config origin

## Runtime/export considerations

### `conch export`

If added later, it should share as much as possible with `init`:

- same config parsing
- same predicate parsing
- same graph ordering
- same conflict analysis
- same provider rendering logic where possible

The key difference would be command UX, not core semantics.

### Runtime focus remains secondary

The project should continue to treat static generation as the primary mode.
Runtime/export features should not complicate the core architecture prematurely.
