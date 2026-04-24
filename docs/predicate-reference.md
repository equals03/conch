# Predicate reference

Predicates are used by both `when` and `requires`.

## Semantics

- each predicate list is an AND
- `!` negates a single predicate atom
- predicates are parsed by `conch`
- during `check`, `init`, and `explain`, predicates remain descriptive and are not evaluated against the host
- during `build`, `shell`, `command`, `file`, `dir`, `os`, and `hostname` are folded on the current machine, while `interactive`, `login`, and all `env:*` predicates remain runtime checks
- any predicates that survive are evaluated by the generated shell code when sourced

## Supported atoms

### `interactive`

Session is interactive.

### `login`

Session is a login shell.

### `shell:<name>`

Target shell matches the given shell name.
Examples:

- `shell:fish`
- `shell:bash`

### `command:<name>`

A command exists in `PATH`.
Example:

- `command:nvim`

### `env:<name>`

An environment variable exists and is non-empty.
Example:

- `env:EDITOR`

### `env:<name>=<value>`

An environment variable equals a literal value.
Example:

- `env:VISUAL=nvim`

### `file:<path>`

A path exists (file or directory).
Example:

- `file:~/.config/nvim/init.lua`
- `file:~/.config/nvim`

### `dir:<path>`

A path exists and is a directory.
Example:

- `dir:~/.config/nvim`

### `os:<name>`

Operating system matches a name.
Example:

- `os:linux`

### `hostname:<name>`

Hostname matches a name.
Example:

- `hostname:workstation`

## Negation examples

- `!interactive`
- `!command:nvim`
- `!env:EDITOR`

## Examples

```toml
when = ["interactive", "shell:fish"]
requires = ["command:nvim"]
when = ["!env:EDITOR"]
requires = ["file:~/.local/bin/nvim"]
requires = ["dir:~/.local/bin"]
```

## v1 notes

- conch validates predicate syntax but does not evaluate predicates during check/init/explain
- `build` folds only `shell`, `command`, `file`, `dir`, `os`, and `hostname`, and does so on the current machine
- providers compile any remaining predicates into shell-native checks
- conflict detection does not attempt predicate satisfiability or mutual-exclusion reasoning
