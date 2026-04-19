# Config reference

## Root shape

Conch v1 defines configuration under `app.<id>` tables.

```toml
[blocks.nvim]
when = ["interactive"]
requires = ["command:nvim"]
after = ["editor"]

[blocks.nvim.env]
EDITOR = "nvim"

[blocks.nvim.alias]
vim = "nvim"

[blocks.nvim.path]
prepend = ["~/.local/bin"]

[blocks.nvim.shell.fish.alias]
v = "nvim"
```

## App fields

### `when = [..]`

A list of predicates describing session/context guards.
The list is an AND.
Rendered as shell-native guards at source time.
See `docs/predicate-reference.md` for supported predicate kinds such as `interactive`, `command:...`, `file:...`, and `dir:...`.

### `requires = [..]`

A list of predicates describing system/resource guards.
The list is an AND.
Rendered as shell-native guards at source time.
See `docs/predicate-reference.md` for supported predicate kinds such as `interactive`, `command:...`, `file:...`, and `dir:...`.

### `before = [..]`

Declares that this app should be emitted before the listed apps.

### `after = [..]`

Declares that this app should be emitted after the listed apps.

### `[blocks.<id>.env]`

Environment exports. Values may be strings, booleans, or integers. Providers decide how to render each scalar for the target shell.

### `[blocks.<id>.alias]`

String key/value alias definitions.

### `[blocks.<id>.path]`

Structured PATH operations.

Supported keys:

- `prepend = [..]`
- `append = [..]`
- `move_front = [..]`
- `move_back = [..]`

### `[blocks.<id>.shell.<name>]`

Optional shell-specific overrides.
Supported v1 target shells:

- `fish`
- `bash`

Unknown shell sections are allowed and ignored by v1 providers unless a future provider targets that shell.

#### `source_lines = [..]` (under `[blocks.<id>.shell.<name>]`)

A list of strings emitted **verbatim** by the target shell provider, after structured `env`, `alias`, and `path` actions from the same app block. Each string is one or more physical lines; conch does not validate, rewrite, or sandbox this text (same trust model as editing an rc file by hand).

Use `when` / `requires` on the app to guard these lines (for example `interactive` and `command:starship`).

When guards apply, each logical line (every TOML array entry, and every line break inside an entry) is emitted with the same block indentation as other actions in that app block.

## Ordering and conflicts

- apps are ordered by the DAG from `before` / `after`
- env/alias writes are checked conservatively
- if two apps write the same env/alias key without a graph ordering relationship, conch reports a conflict
- if an order exists, later blocks are emitted later

## App id rules

- must be non-empty
- must not have leading or trailing whitespace

## Notes

- predicates are not evaluated by the CLI
- providers render predicates into shell-native guards
- path replacement as a raw string is intentionally not supported in v1
