# Conch architecture

## Overview

`conch` compiles app-centric TOML configuration into shell-native output.

The key design choice is that predicates such as `when` and `requires` are **not** evaluated by the CLI. They are parsed, validated, and rendered as shell-native guards that are evaluated when the generated file is sourced.

## Data flow

1. **Config parsing**
   - `src/config/`
   - parses TOML into a raw model and converts it into a typed config model
2. **Predicate parsing**
   - `src/predicate.rs`
   - validates predicate syntax only
3. **Graph ordering**
   - `src/graph.rs`
   - builds a DAG across declared apps using `before` / `after`
   - detects cycles and unknown references
4. **Resolution**
   - `src/resolve.rs`
   - topologically sorts app blocks
   - applies shell overrides for the target shell
   - detects conservative env/alias write conflicts
   - produces shell-neutral ordered guarded blocks
   - records explainability data
5. **Explain rendering**
   - `src/explain.rs`
   - turns shell-neutral report data into readable output for `conch explain`
6. **Provider rendering**
   - `src/provider/`
   - renders guards and actions into Fish or Bash syntax

## Core principles

### 1. App-centric config

Configuration is organized around applications/tools rather than shell primitives.

### 2. Deferred shell-time guards

`conch` does not inspect the current host to decide whether an app is active.
Providers render guards such as:

- Fish: `status is-interactive`, `command -q nvim`
- Bash: `[[ $- == *i* ]]`, `command -v nvim >/dev/null 2>&1`

### 3. Shell-neutral core

The core knows about:

- ordering
- predicates as syntax trees
- env/alias/path actions
- conservative conflict rules

The core does **not** know Fish/Bash syntax.

### 4. Conservative conflict detection

If two apps write the same env/alias key and the graph does not order them, resolution fails.
Conch does not try to prove that guards are mutually exclusive in v1.

## Intermediate representation

The IR is an ordered list of app blocks.
Each block contains:

- app id
- parsed `when` predicates
- parsed `requires` predicates
- ordered actions

This makes provider rendering straightforward while preserving shell-neutral semantics.

## Explain model

`conch explain` reports static information only:

- app graph order
- app guards
- ordered env writers
- ordered alias writers
- path operations and their source app

It does not claim which apps are active on the current machine.

## Why this structure scales

This split is intended to make new providers additive.
Adding a new shell should primarily require:

- guard rendering
- action rendering
- documenting shell-specific capability tradeoffs

without changing the core resolver.
