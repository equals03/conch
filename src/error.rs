use std::path::PathBuf;

use thiserror::Error;

/// Top-level error type for the `conch` CLI and library.
#[derive(Debug, Error)]
pub enum ConchError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse TOML config: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("failed to parse YAML config: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("failed to parse JSON config: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("invalid predicate: {0}")]
    PredicateParse(String),

    #[error("invalid graph: {0}")]
    Graph(String),

    #[error("merge conflict: {0}")]
    MergeConflict(String),

    #[error("invalid configuration: {0}")]
    Validation(String),

    #[error("config file not found: {0}")]
    ConfigNotFound(PathBuf),

    #[error("default config file not found; searched XDG config locations: {0}")]
    DefaultConfigNotFound(String),
}
