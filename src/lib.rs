//! `conch` — declarative shell-configuration compiler.
//!
//! ## Architecture
//!
//! 1. **Config** — TOML, YAML, or JSON is parsed into [`config::RawConfig`].
//! 2. **Predicates** — [`predicate`] parses `when` / `requires` syntax.
//! 3. **Env identifiers** — the internal `env_name` module validates names for `env:` predicates and `${env:...}` interpolation.
//! 4. **Graph** — [`graph`] builds a DAG on declared blocks using `before` / `after`.
//! 5. **Resolution** — [`resolve`] topologically sorts blocks, detects
//!    conservative write conflicts, records explainability data, and emits a
//!    shell-neutral guarded [`ir::ResolvedIr`].
//! 6. **Explain** — [`explain`] renders human-readable resolution details.
//! 7. **Providers** — [`provider`] renders guarded [`ir::ResolvedIr`] to Fish
//!    or Bash with shell-native predicate checks.

pub mod build;
pub mod cli;
pub mod config;
mod env_name;
pub mod error;
pub mod explain;
pub mod graph;
pub mod ir;
pub mod predicate;
pub mod provider;
pub mod resolve;

pub use build::{detect_hostname, detect_os, HostFoldContext};
pub use error::ConchError;
