<div align="center">

# conch

**Declarative shell configuration, compiled to Fish or Bash**

[![CI](https://img.shields.io/github/actions/workflow/status/equals03/conch/ci.yml?style=flat-square&label=CI)](https://github.com/equals03/conch/actions)
[![Rust MSRV](https://img.shields.io/badge/MSRV-1.70-orange?style=flat-square)](./Cargo.toml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue?style=flat-square)](./LICENCE)

[Overview](#overview) · [Features](#features) · [Quickstart](#quickstart) · [Example configs](#example-configs) · [CLI](#cli) · [Semantics](#semantics) · [Documentation](#documentation) · [Troubleshooting](#troubleshooting)

</div>

## Overview

`conch` is a **declarative shell-configuration compiler**. You describe setup once in TOML, YAML, or JSON as **blocks** (per tool, role, or section), and `conch` emits shell-native output for a target shell.

> [!IMPORTANT]
> `conch` does **not** evaluate `when` / `requires` during `check`, `init`, or `explain`. It validates predicates, orders blocks, detects conflicts, and emits **guarded shell** that your shell evaluates when you `source` the generated file.

At a high level it:

1. Parses config
2. Validates predicate syntax and ordering relationships
3. Orders blocks with the block graph
4. Checks env/alias write conflicts conservatively
5. Emits guarded shell blocks

## Features

- Block-oriented config under `blocks.<id>`
- `when`, `requires`, `before`, `after`
- `env` (strings, booleans, integers, or `{ raw = "..." }` for provider-specific fragments), `alias`, and structured `path` (`prepend` / `append` / `move_front` / `move_back`)
- Shell-specific tables under `shell.fish` / `shell.bash` for `env`, `alias`, `path`, and verbatim `source_lines`
- Fish and Bash providers
- `check`, `init`, and `explain` commands
- Deterministic ordering, explicit conflict detection, fixture/golden tests

> [!NOTE]
> **v1 scope:** Fish and Bash only. **Non-goals for v1:** arbitrary shell code blocks, portable shell functions, plugins, RC auto-install, hot reload, runtime caches, or daemons.

## Quickstart

The repository ships the **same** illustrative config as [`examples/conch.toml`](./examples/conch.toml), [`examples/conch.yaml`](./examples/conch.yaml), and [`examples/conch.json`](./examples/conch.json). Start from whichever serialisation you prefer.

If `--config` is omitted, `conch` searches XDG config locations in this order:

1. `${XDG_CONFIG_HOME}/conch.{toml,yaml,yml,json}`
2. `${XDG_CONFIG_HOME}/conch/config.{toml,yaml,yml,json}`
3. each directory in `${XDG_CONFIG_DIRS}` (default `/etc/xdg`) using the same two layouts

If `XDG_CONFIG_HOME` is unset, `conch` falls back to `~/.config`.

Minimal shape (see the files for the full tour):

```toml
[blocks.core]
[blocks.core.path]
prepend = ["~/.local/bin"]

[blocks.nvim]
when = ["interactive"]
requires = ["command:nvim"]
after = ["editor"]

[blocks.nvim.shell.fish.alias]
v = "nvim"
```

Generate and install:

```bash
conch init fish --config examples/conch.toml > ~/.config/conch/config.fish
# or YAML / JSON:
conch init fish --config examples/conch.yaml > ~/.config/conch/config.fish
```

```bash
conch init bash --config examples/conch.toml > ~/.config/conch/config.bash
conch init bash --config examples/conch.json > ~/.config/conch/config.bash
```

### Source the output

**Fish** — in `~/.config/fish/config.fish`:

```fish
source ~/.config/conch/config.fish
```

**Bash** — in `~/.bashrc`:

```bash
source ~/.config/conch/config.bash
```

### Inline load (rebuild every shell start)

If you prefer to run `conch` on each login so the shell always loads freshly compiled output (at the cost of startup time), call `init` from your rc file and feed it straight into the shell. Use the **bash** output in bash and the **fish** output in fish.

**Fish** — in `~/.config/fish/config.fish`:

```fish
conch init fish | source
```

**Bash** — in `~/.bashrc`:

```bash
eval "$(conch init bash)"
```

Keep `conch` on `PATH`, and ensure anything besides the script goes to stderr so the streamed output stays valid shell.

## Example configs

The bundled `examples/conch.*` files are intentionally verbose: they show how blocks compose for real tooling (editors, PATH, prompt integrations, directory hopping, and optional pickers). In outline they cover:

- **Ordering** — `after` chains (for example `core` → `editor` → `nvim`).
- **Predicates** — `when` (such as `interactive`, `login`, `os:linux`) and `requires` (such as `command:…`, `dir:…`, and `!command:…` for mutual exclusion).
- **Structured `PATH`** — `prepend`, `append`, and `move_front`.
- **Env** — plain strings, boolean/integer flags, and `{ raw = "…" }` for shell-specific values (see `shell-ident`).
- **Shell overrides** — `shell.fish` / `shell.bash` for extra `alias` / `env` / `path`, plus `source_lines` emitted verbatim after structured fields in that block.
- **Third-party init patterns** — guarded `source_lines` for Starship, zoxide, direnv, fzf, and similar tools.

Use them as a cookbook: copy the blocks you need, drop the rest, and tighten `requires` to match your machine.

## Build and install

```bash
cargo build
cargo test
```

Release binary:

```bash
cargo build --release
# ./target/release/conch
```

Optional: this repo includes a Nix flake with a `devShell` (`nix develop`) for a Rust toolchain.

## CLI

### `check` — validate config

`--config` is optional when your config lives in a discovered XDG location. The examples below pass `--config` to point at the bundled example files.

```bash
conch check --config examples/conch.toml
conch check --config examples/conch.yaml
conch check fish --config examples/conch.toml
conch check bash --config examples/conch.json
```

Static validation only:

- Omit the shell argument to validate **both** Fish and Bash.
- With `fish` or `bash`, only that target is checked.
- Does **not** evaluate `when` / `requires` against the current host.
- Shell overrides apply only for the shell under validation.
- Unknown override sections (e.g. `[blocks.foo.shell.zsh]`) are allowed and ignored by v1 providers.
- Duplicate ordering edges are ignored.

### `init` — emit shell

```bash
conch init fish
conch init bash
# or point at a specific file:
conch init fish --config examples/conch.toml
conch init bash --config examples/conch.json
```

Predicates become shell-native guards, for example:

- Fish: `command -q nvim`
- Bash: `command -v nvim >/dev/null 2>&1`

### `explain` — inspect resolution

```bash
conch explain fish
conch explain bash
# or point at a specific file:
conch explain fish --config examples/conch.toml
conch explain bash --config examples/conch.json
conch explain fish --color never --config examples/conch.yaml
```

Shows block order, guards (`when` / `requires`), per-block contributions, ordered env/alias writers, and `PATH` operations. It does **not** claim which blocks are active on your machine.

> [!TIP]
> `--color auto|always|never` controls ANSI output (default: color when stdout is a TTY).

## Semantics

### Predicates

Each block may define:

- `when` — session/context predicates
- `requires` — system/resource predicates

Both lists use the same syntax; entries are **AND**'d.

```toml
when = ["interactive", "shell:fish"]
requires = ["command:nvim"]
when = ["!env:EDITOR"]
```

`conch` parses and validates; providers render guards; the shell evaluates them at source time. See [`docs/predicate-reference.md`](./docs/predicate-reference.md).

### Graph ordering

Use `before` / `after` to constrain order:

```toml
[blocks.base]

[blocks.editor]
after = ["base"]

[blocks.nvim]
after = ["editor"]
```

Static order: `base` → `editor` → `nvim`. Unknown block ids are errors; cycles are reported with a path.

### Conflicts

Env and alias writes are checked conservatively: two blocks writing the same key without a graph ordering edge is an error.

```toml
[blocks.a.alias]
vim = "nvim"

[blocks.b.alias]
vim = "hx"
```

Add ordering so the winner is explicit, e.g. `[blocks.a] before = ["b"]` so `b` is emitted later.

> [!NOTE]
> v1 does **not** try to prove predicate mutual exclusivity; conflict checking stays intentionally conservative.

## Sample Fish output

This is an excerpt from `conch init fish --config examples/conch.toml` — your generated file will include every block from your config, in resolved order:

```fish
# Generated by conch for fish.

# block: core
fish_add_path --prepend "$HOME/.local/bin";
fish_add_path --append "/usr/local/sbin";
fish_add_path --move --prepend "$HOME/bin";

# block: demoflags
set -gx ENABLE_CONCH_DEMO true;
set -gx CONCH_DEMO_RETRIES 3;

# block: editor
set -gx EDITOR "nvim";
set -gx VISUAL "nvim";

# block: shell-ident
set -gx SHELL (command -v fish);

if status is-interactive

    # block: convenience
    alias cd..="cd ..";
    alias la="ls -la";
end

if status is-login

    # block: login-path
    fish_add_path --append "/usr/local/bin";
end

if status is-interactive

    # block: nvim
    if command -q -- "nvim"
        alias vim="nvim";
        alias v="nvim";
    end

    # block: linux-tip
    if test (uname -s | string lower) = "linux"
        set -gx CONCH_ON_LINUX true;
    end

    # block: bat
    if command -q -- "bat"
        alias cat="bat --style=plain --paging=never";
    end

    # …further blocks (eza, starship, zoxide, direnv, fzf, tv, stow-hint) continue here…
end
```

## Provider notes

| Shell | Status | Notes |
| ----- | ------ | ----- |
| Fish  | Supported | Uses `fish_add_path`; `move_front` / `move_back` map to `--move`. |
| Bash  | Supported | `move_front` / `move_back` are approximated as prepend/append in v1 (noted in generated comments). |
| Zsh, Nushell | Planned | — |

## Project layout

- `src/config/` — raw and typed config models
- `src/predicate.rs` — predicate parsing
- `src/graph.rs` — block DAG and topological sort
- `src/resolve.rs` — ordering, conflict analysis, explain data
- `src/explain.rs` — human-readable explain output
- `src/ir/` — ordered guarded block IR
- `src/provider/` — shell renderers

## Documentation

- [`docs/architecture.md`](./docs/architecture.md)
- [`docs/config-reference.md`](./docs/config-reference.md)
- [`docs/predicate-reference.md`](./docs/predicate-reference.md)
- [`docs/provider-design.md`](./docs/provider-design.md)
- [`docs/future-considerations.md`](./docs/future-considerations.md)

## Troubleshooting

| Message | What to do |
| ------- | ---------- |
| `default config file not found; searched XDG config locations: ...` | Create a config at one of the listed paths, or pass `--config` explicitly. Ensure `HOME` / `XDG_CONFIG_HOME` resolve as you expect. |
| `merge conflict: ... does not order them` | Two blocks write the same env/alias key without a graph relationship. Add `before` / `after`. |
| `invalid graph: cycle detected ...` | Remove or change a `before` / `after` edge. |
| `invalid predicate: ...` | Fix predicate syntax; see [`docs/predicate-reference.md`](./docs/predicate-reference.md). |
| Block “did not apply” | Guards run at source time. Use `conch explain fish` or `conch explain bash` to inspect order and guards. |
