//! `conch` — declarative shell-configuration compiler.
//!
//! ## Architecture
//!
//! 1. **Config** — TOML, YAML, or JSON is parsed into [`config::RawConfig`].
//! 2. **Predicates** — [`predicate`] parses `when` / `requires` syntax.
//! 3. **Graph** — [`graph`] builds a DAG on declared blocks using `before` / `after`.
//! 4. **Resolution** — [`resolve`] topologically sorts blocks, detects
//!    conservative write conflicts, records explainability data, and emits a
//!    shell-neutral guarded [`ir::ResolvedIr`].
//! 5. **Explain** — [`explain`] renders human-readable resolution details.
//! 6. **Providers** — [`provider`] renders guarded [`ir::ResolvedIr`] to Fish
//!    or Bash with shell-native predicate checks.

pub mod build;
pub mod cli;
pub mod config;
pub mod error;
pub mod explain;
pub mod graph;
pub mod ir;
pub mod predicate;
pub mod provider;
pub mod resolve;

pub use error::ConchError;
