# Provider design notes

## Purpose

Providers take the shell-neutral ordered app-block IR and render shell-native output.

The core resolver owns:

- config parsing
- predicate parsing
- app graph ordering
- conservative conflict detection
- explainability data

Providers own only:

- guard rendering for a specific shell
- action rendering for a specific shell
- shell-specific tradeoff documentation
- safe provider-side guard shaping such as hoisting adjacent shared `interactive` / `login` checks without changing app order
- verbatim emission of `SourceLines` actions (conch does not validate shell syntax; authors use `when` / `requires` like any other app block)

## Current providers

### Fish

- Renders guards with Fish-native commands such as:
  - `status is-interactive`
  - `status is-login`
  - `command -q <name>`
- Hoists adjacent shared `interactive` / `login` guard prefixes when safe, then renders residual guards per app block.
- Uses `fish_add_path` for path operations.
- Supports precise `move_front` / `move_back` with `fish_add_path --move`.

### Bash

- Renders guards with Bash expressions such as:
  - `[[ $- == *i* ]]`
  - `shopt -q login_shell`
  - `command -v <name> >/dev/null 2>&1`
- Hoists adjacent shared `interactive` / `login` guard prefixes when safe, then renders residual guards per app block.
- Renders PATH directly with `export PATH=...`.
- In v1, `move_front` and `move_back` are approximated as prepend/append.
- This approximation is intentional for v1 to keep the generated output simple and readable.

## Provider capability flags

Providers expose lightweight capability metadata:

- `precise_path_move`
- `native_path_helper`

These flags are not used by the resolver yet, but they give future work a place to describe provider tradeoffs without leaking shell semantics into the core.

## Why the core stays shell-neutral

The core should not know how Fish or Bash checks for commands, login shells, env vars, or path mutation.
That logic belongs in providers.

This keeps future providers additive:

- the resolver can stay stable
- new shells only need rendering + capability decisions

## Zsh provider sketch

A future Zsh provider would likely:

- render command checks with `command -v`
- use `[[ -o interactive ]]` or equivalent interactive checks
- use Zsh-native alias/export syntax close to Bash
- decide whether PATH moves should remain approximate or use array-based PATH manipulation

Expected work:

- mostly provider-side rendering
- little or no resolver change

## Nushell provider sketch

A future Nushell provider is the more interesting case because:

- aliases and env mutation syntax differ much more from POSIX shells
- PATH may be structured rather than purely string-based
- login/interactive detection may require different guard idioms

Expected work:

- provider-specific rendering layer
- possible additional provider capability flags
- maybe a small IR extension if Nushell needs a path or alias representation that is meaningfully different

## Known IR gaps to watch

The current IR is good enough for Fish and Bash, but future shells may pressure it in these areas:

1. PATH semantics
   - some shells may want a more structured PATH model than ordered imperative ops
2. alias/function distinction
   - some shells may blur alias and function portability differently
3. shell constants
   - `shell:<name>` currently becomes a provider-side constant true/false
4. richer diagnostics
   - if providers need better source mapping, the IR may need source spans later

For v1, none of these require resolver changes yet.
