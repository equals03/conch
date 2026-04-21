# Future considerations

This document closes out the current plan by recording near-term design considerations without prematurely implementing them.

## CLI considerations

### `conch check <fish|bash>`

Optional positional shell argument; already supported.

### `conch init --output <path>`

Likely worthwhile later as a convenience wrapper around stdout redirection.
Not needed for the core compiler semantics.

### Config search conventions

Possible future behaviour:

- current directory `conch.toml`
- XDG config locations
- explicit project config conventions

v1 stays explicit with `--config` plus a simple default file name.

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
