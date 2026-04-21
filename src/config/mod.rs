//! Configuration models.
//!
//! `RawConfig` is the serde/config-format boundary.
//! `Config` is the typed domain model used by graph construction and resolution.

mod raw;

use std::fmt;

use indexmap::IndexMap;
use serde::de::{self, Deserializer, MapAccess, Unexpected, Visitor};
use serde::Deserialize;

use crate::error::ConchError;

/// Shared [`Visitor::expecting`] / [`de::Error::invalid_type`] text for [`EnvValue`].
const ENV_VALUE_EXPECTING: &str =
    "a string, integer, boolean env value, or a table `{ raw = \"...\" }`";

/// Maximum absolute value accepted when coercing a deserialised floating-point scalar to an integer
/// env value.
///
/// Uses `2^53 - 1` (ECMA-262 `Number.MAX_SAFE_INTEGER`). `visit_f32` / `visit_f64` are not JSON-specific:
/// any deserializer that yields IEEE-754 floats is subject to these limits. Adjacent integers need
/// not map to distinct values once the magnitude reaches `2^53` (for example `9007199254740992.0` and
/// `9007199254740993.0` compare equal).
const ENV_VALUE_F64_MAX_ABS_INT: f64 = 9_007_199_254_740_991.0;

fn env_value_reject_float<E: de::Error>(value: f64) -> Result<EnvValue, E> {
    Err(de::Error::invalid_type(
        Unexpected::Float(value),
        &ENV_VALUE_EXPECTING,
    ))
}

pub use raw::{
    BlockConfigToml, InitConfigToml, InitGuardToml, PathSpecToml, RawConfig, ShellOverridesToml,
    SourceEntryFieldsToml, SourceEntryToml,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvValue {
    String(String),
    Bool(bool),
    Integer(String),
    /// Right-hand side emitted verbatim for the target shell (no quoting or escaping).
    Raw(String),
}

impl EnvValue {
    pub fn as_string(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Bool(value) => value.to_string(),
            Self::Integer(value) => value.clone(),
            Self::Raw(value) => value.clone(),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::String(value) => format!("{value:?}"),
            Self::Bool(value) => value.to_string(),
            Self::Integer(value) => value.clone(),
            Self::Raw(value) => format!("raw({value:?})"),
        }
    }
}

impl From<String> for EnvValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for EnvValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<bool> for EnvValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for EnvValue {
    fn from(value: i64) -> Self {
        Self::Integer(value.to_string())
    }
}

impl From<u64> for EnvValue {
    fn from(value: u64) -> Self {
        Self::Integer(value.to_string())
    }
}

impl<'de> Deserialize<'de> for EnvValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EnvValueVisitor;

        impl<'de> Visitor<'de> for EnvValueVisitor {
            type Value = EnvValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(ENV_VALUE_EXPECTING)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(EnvValue::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(EnvValue::Integer(value.to_string()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(EnvValue::Integer(value.to_string()))
            }

            fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_f64(f64::from(value))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if !value.is_finite() {
                    return env_value_reject_float(value);
                }
                if value.abs() > ENV_VALUE_F64_MAX_ABS_INT {
                    return env_value_reject_float(value);
                }
                let as_i64 = value as i64;
                if (as_i64 as f64) != value {
                    return env_value_reject_float(value);
                }
                Ok(EnvValue::Integer(as_i64.to_string()))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(EnvValue::String(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(EnvValue::String(value))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut raw: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "raw" => {
                            if raw.is_some() {
                                return Err(de::Error::duplicate_field("raw"));
                            }
                            raw = Some(map.next_value()?);
                        }
                        other => {
                            return Err(de::Error::unknown_field(other, &["raw"]));
                        }
                    }
                }
                let Some(value) = raw else {
                    return Err(de::Error::missing_field("raw"));
                };
                Ok(EnvValue::Raw(value))
            }
        }

        deserializer.deserialize_any(EnvValueVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceEntry {
    File(String),
    Command(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub init: InitConfig,
    pub blocks: IndexMap<String, BlockConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitConfig {
    pub guard: InitGuardConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitGuardConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockConfig {
    pub when: Vec<String>,
    pub requires: Vec<String>,
    pub before: Vec<String>,
    pub after: Vec<String>,
    pub env: IndexMap<String, EnvValue>,
    pub alias: IndexMap<String, String>,
    pub path: PathSpec,
    pub source: Vec<SourceEntry>,
    pub shell: IndexMap<String, ShellOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellOverride {
    pub env: IndexMap<String, EnvValue>,
    pub alias: IndexMap<String, String>,
    pub path: PathSpec,
    pub source: Vec<SourceEntry>,
    pub source_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathSpec {
    pub prepend: Vec<String>,
    pub append: Vec<String>,
    pub move_front: Vec<String>,
    pub move_back: Vec<String>,
}

impl TryFrom<&RawConfig> for Config {
    type Error = ConchError;

    fn try_from(raw: &RawConfig) -> Result<Self, Self::Error> {
        if raw.blocks.is_empty() {
            return Err(ConchError::Validation(
                "config must define at least one block entry under `blocks.<id>`".into(),
            ));
        }

        let mut blocks = IndexMap::new();
        for (block_id, block) in &raw.blocks {
            validate_block_id(block_id)?;
            blocks.insert(
                block_id.clone(),
                block_config_from_toml(block_id, block.clone())?,
            );
        }

        let config = Self {
            init: raw.init.clone().into(),
            blocks,
        };
        validate_init_guard_reserved_env_keys(&config)?;
        Ok(config)
    }
}

fn validate_block_id(block_id: &str) -> Result<(), ConchError> {
    if block_id.trim().is_empty() {
        return Err(ConchError::Validation("block ids must be non-empty".into()));
    }

    if block_id != block_id.trim() {
        return Err(ConchError::Validation(format!(
            "block id `{block_id}` must not have leading or trailing whitespace"
        )));
    }

    Ok(())
}

fn block_config_from_toml(
    block_id: &str,
    value: BlockConfigToml,
) -> Result<BlockConfig, ConchError> {
    Ok(BlockConfig {
        when: value.when,
        requires: value.requires,
        before: value.before,
        after: value.after,
        env: value.env,
        alias: value.alias,
        path: value.path.into(),
        source: convert_source_entries(value.source, &format!("block `{block_id}`"))?,
        shell: value
            .shell
            .into_iter()
            .map(|(shell, override_cfg)| {
                let shell_override = shell_override_from_toml(block_id, &shell, override_cfg)?;
                Ok::<_, ConchError>((shell, shell_override))
            })
            .collect::<Result<IndexMap<_, _>, _>>()?,
    })
}

fn shell_override_from_toml(
    block_id: &str,
    shell: &str,
    value: ShellOverridesToml,
) -> Result<ShellOverride, ConchError> {
    Ok(ShellOverride {
        env: value.env,
        alias: value.alias,
        path: value.path.into(),
        source: convert_source_entries(
            value.source,
            &format!("block `{block_id}` shell override `{shell}`"),
        )?,
        source_lines: value.source_lines,
    })
}

fn convert_source_entries(
    entries: Vec<SourceEntryToml>,
    scope: &str,
) -> Result<Vec<SourceEntry>, ConchError> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| source_entry_from_toml(entry, scope, index + 1))
        .collect()
}

fn source_entry_from_toml(
    entry: SourceEntryToml,
    scope: &str,
    position: usize,
) -> Result<SourceEntry, ConchError> {
    match entry {
        SourceEntryToml::File(path) => validate_source_file(path, scope, position),
        SourceEntryToml::Structured(fields) => match (fields.file, fields.command) {
            (Some(path), None) => validate_source_file(path, scope, position),
            (None, Some(command)) => validate_source_command(command, scope, position),
            (Some(_), Some(_)) => Err(ConchError::Validation(format!(
                "{scope} source entry #{position} must set exactly one of `file` or `command`"
            ))),
            (None, None) => Err(ConchError::Validation(format!(
                "{scope} source entry #{position} must set one of `file` or `command`"
            ))),
        },
    }
}

fn validate_source_file(
    path: String,
    scope: &str,
    position: usize,
) -> Result<SourceEntry, ConchError> {
    if path.trim().is_empty() {
        return Err(ConchError::Validation(format!(
            "{scope} source entry #{position} file path must not be empty"
        )));
    }
    Ok(SourceEntry::File(path))
}

fn validate_source_command(
    command: Vec<String>,
    scope: &str,
    position: usize,
) -> Result<SourceEntry, ConchError> {
    if command.is_empty() {
        return Err(ConchError::Validation(format!(
            "{scope} source entry #{position} command must not be empty"
        )));
    }
    if command[0].trim().is_empty() {
        return Err(ConchError::Validation(format!(
            "{scope} source entry #{position} command name must not be empty"
        )));
    }
    Ok(SourceEntry::Command(command))
}

fn validate_init_guard_reserved_env_keys(config: &Config) -> Result<(), ConchError> {
    if !config.init.guard.enabled {
        return Ok(());
    }

    for (block_id, block) in &config.blocks {
        validate_reserved_env_scope(block_id, None, &block.env)?;
        for (shell, override_cfg) in &block.shell {
            validate_reserved_env_scope(block_id, Some(shell.as_str()), &override_cfg.env)?;
        }
    }

    Ok(())
}

fn validate_reserved_env_scope(
    block_id: &str,
    shell: Option<&str>,
    env: &IndexMap<String, EnvValue>,
) -> Result<(), ConchError> {
    for key in env.keys() {
        if is_reserved_init_guard_env_key(key) {
            let scope = match shell {
                Some(shell) => format!("block `{block_id}` shell override `{shell}`"),
                None => format!("block `{block_id}`"),
            };
            return Err(ConchError::Validation(format!(
                "{scope} cannot set reserved env key `{key}` when `[init.guard] enabled = true`; conch emits that variable automatically"
            )));
        }
    }

    Ok(())
}

fn is_reserved_init_guard_env_key(key: &str) -> bool {
    matches!(
        key,
        "__CONCH_SOURCED" | "__CONCH_FISH_SOURCED" | "__CONCH_BASH_SOURCED"
    )
}

impl From<InitConfigToml> for InitConfig {
    fn from(value: InitConfigToml) -> Self {
        Self {
            guard: value.guard.into(),
        }
    }
}

impl From<InitGuardToml> for InitGuardConfig {
    fn from(value: InitGuardToml) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

impl From<PathSpecToml> for PathSpec {
    fn from(value: PathSpecToml) -> Self {
        Self {
            prepend: value.prepend,
            append: value.append,
            move_front: value.move_front,
            move_back: value.move_back,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_raw_to_typed_config() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.nvim]
            when = ["interactive"]
            requires = ["command:nvim"]
            before = ["editor"]

            [blocks.nvim.env]
            EDITOR = "nvim"
            ENABLE_TRACE = true
            RETRIES = 3

            [blocks.nvim.alias]
            vim = "nvim"

            [blocks.nvim.path]
            prepend = ["~/.local/bin"]
            "#,
        )
        .unwrap();

        let config = Config::try_from(&raw).unwrap();
        assert_eq!(config.init, InitConfig::default());
        let nvim = &config.blocks["nvim"];
        assert_eq!(nvim.when, vec!["interactive"]);
        assert_eq!(nvim.requires, vec!["command:nvim"]);
        assert_eq!(nvim.before, vec!["editor"]);
        assert_eq!(nvim.env["EDITOR"], EnvValue::from("nvim"));
        assert_eq!(nvim.env["ENABLE_TRACE"], EnvValue::from(true));
        assert_eq!(nvim.env["RETRIES"], EnvValue::from(3_i64));
        assert_eq!(nvim.alias["vim"], "nvim");
        assert_eq!(nvim.path.prepend, vec!["~/.local/bin"]);
    }

    #[test]
    fn converts_structured_source_entries() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.baile]
            source = ["~/.baile-env", { command = ["starship", "init", "{shell}"] }]

            [blocks.baile.shell.fish]
            source = [{ file = "~/.config/fish/local.fish" }]
            source_lines = ["echo sourced"]
            "#,
        )
        .unwrap();

        let config = Config::try_from(&raw).unwrap();
        let baile = &config.blocks["baile"];
        assert_eq!(
            baile.source,
            vec![
                SourceEntry::File("~/.baile-env".into()),
                SourceEntry::Command(vec!["starship".into(), "init".into(), "{shell}".into(),]),
            ]
        );
        assert_eq!(
            baile.shell["fish"].source,
            vec![SourceEntry::File("~/.config/fish/local.fish".into())]
        );
        assert_eq!(baile.shell["fish"].source_lines, vec!["echo sourced"]);
    }

    #[test]
    fn parses_raw_env_table_in_toml() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.demo.env]
            LEAN_CTX_BIN = { raw = "$(command -v lean-ctx)" }
            "#,
        )
        .unwrap();

        let demo = &raw.blocks["demo"];
        assert_eq!(
            demo.env["LEAN_CTX_BIN"],
            EnvValue::Raw("$(command -v lean-ctx)".into())
        );
    }

    #[test]
    fn env_value_f64_max_abs_int_matches_ecma_max_safe_integer() {
        assert_eq!(ENV_VALUE_F64_MAX_ABS_INT as i64, 9_007_199_254_740_991);
        assert_eq!(
            (ENV_VALUE_F64_MAX_ABS_INT as i64) as f64,
            ENV_VALUE_F64_MAX_ABS_INT
        );
    }

    #[test]
    fn rejects_invalid_block_ids() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks." bad "]
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("leading or trailing whitespace"));
    }

    #[test]
    fn converts_init_guard_settings() {
        let raw: RawConfig = toml::from_str(
            r#"
            [init.guard]
            enabled = true

            [blocks.demo.env]
            EDITOR = "nvim"
            "#,
        )
        .unwrap();

        let config = Config::try_from(&raw).unwrap();
        assert!(config.init.guard.enabled);
    }

    #[test]
    fn rejects_reserved_env_keys_when_init_guard_is_enabled() {
        let raw: RawConfig = toml::from_str(
            r#"
            [init.guard]
            enabled = true

            [blocks.demo.env]
            __CONCH_SOURCED = "1"
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err
            .to_string()
            .contains("reserved env key `__CONCH_SOURCED`"));
    }

    #[test]
    fn rejects_reserved_env_keys_in_shell_overrides_when_init_guard_is_enabled() {
        let raw: RawConfig = toml::from_str(
            r#"
            [init.guard]
            enabled = true

            [blocks.demo.shell.fish.env]
            __CONCH_FISH_SOURCED = "1"
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("shell override `fish`"));
        assert!(err
            .to_string()
            .contains("reserved env key `__CONCH_FISH_SOURCED`"));
    }

    #[test]
    fn rejects_source_entries_that_set_both_file_and_command() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.demo]
            source = [{ file = "~/.demo", command = ["echo", "demo"] }]
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err
            .to_string()
            .contains("must set exactly one of `file` or `command`"));
    }

    #[test]
    fn rejects_empty_source_commands() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.demo]
            source = [{ command = [] }]
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("command must not be empty"));
    }

    #[test]
    fn rejects_empty_source_file_paths() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.demo]
            source = [""]
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("file path must not be empty"));
    }

    #[test]
    fn rejects_empty_source_command_names() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.demo]
            source = [{ command = ["", "init"] }]
            "#,
        )
        .unwrap();

        let err = Config::try_from(&raw).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("command name must not be empty"));
    }
}
