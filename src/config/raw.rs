//! Raw serde model for `conch` configuration documents.
//!
//! TOML tables like `[blocks.nvim]`, `[blocks.nvim.env]`, and
//! `[blocks.nvim.shell.fish.alias]` merge into one [`BlockConfigToml`] per block id.
//! YAML and JSON deserialize into the same shape.
//! Unknown fields are rejected at parse time via `#[serde(deny_unknown_fields)]` on each raw struct.
//! [`EnvValue`] uses a custom deserializer; unsupported scalar shapes fail with the same
//! expectation string as other `EnvValue` parse errors.

use indexmap::IndexMap;
use serde::Deserialize;

use super::EnvValue;

/// Root document: only `blocks.<id>` entries are defined for v1.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    pub blocks: IndexMap<String, BlockConfigToml>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct BlockConfigToml {
    #[serde(default)]
    pub when: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub env: IndexMap<String, EnvValue>,
    #[serde(default)]
    pub alias: IndexMap<String, String>,
    #[serde(default)]
    pub path: PathSpecToml,
    /// Key: shell name, e.g. `fish`, `bash`.
    #[serde(default)]
    pub shell: IndexMap<String, ShellOverridesToml>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct ShellOverridesToml {
    #[serde(default)]
    pub env: IndexMap<String, EnvValue>,
    #[serde(default)]
    pub alias: IndexMap<String, String>,
    #[serde(default)]
    pub path: PathSpecToml,
    /// Shell-specific lines emitted verbatim by the target provider.
    #[serde(default)]
    pub source_lines: Vec<String>,
}

#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PathSpecToml {
    #[serde(default)]
    pub prepend: Vec<String>,
    #[serde(default)]
    pub append: Vec<String>,
    #[serde(default)]
    pub move_front: Vec<String>,
    #[serde(default)]
    pub move_back: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn assert_unknown_field_error(err: impl std::fmt::Display) {
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("unknown field"),
            "expected unknown-field error, got: {msg}"
        );
    }

    #[test]
    fn parses_nested_block_tables() {
        let raw: RawConfig = toml::from_str(
            r#"
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
            "#,
        )
        .unwrap();

        let nvim = &raw.blocks["nvim"];
        assert_eq!(nvim.when, vec!["interactive"]);
        assert_eq!(nvim.requires, vec!["command:nvim"]);
        assert_eq!(nvim.env["EDITOR"], EnvValue::from("nvim"));
        assert_eq!(nvim.alias["vim"], "nvim");
        assert_eq!(nvim.path.prepend, vec!["~/.local/bin"]);
        assert_eq!(nvim.shell["fish"].alias["v"], "nvim");
    }

    #[test]
    fn parses_non_string_env_scalars_in_toml() {
        let raw: RawConfig = toml::from_str(
            r"
            [blocks.flags.env]
            ENABLE_TRACE = true
            RETRIES = 3

            [blocks.flags.shell.fish.env]
            FEATURE_GATE = false
            DEPTH = 1
            ",
        )
        .unwrap();

        let flags = &raw.blocks["flags"];
        assert_eq!(flags.env["ENABLE_TRACE"], EnvValue::from(true));
        assert_eq!(flags.env["RETRIES"], EnvValue::from(3_i64));
        assert_eq!(
            flags.shell["fish"].env["FEATURE_GATE"],
            EnvValue::from(false)
        );
        assert_eq!(flags.shell["fish"].env["DEPTH"], EnvValue::from(1_i64));
    }

    #[test]
    fn parses_source_lines_in_shell_overrides() {
        let raw: RawConfig = toml::from_str(
            r#"
            [blocks.starship]
            when = ["interactive"]

            [blocks.starship.shell.fish]
            source_lines = ["starship init fish | source"]

            [blocks.starship.shell.bash]
            source_lines = ['eval "$(starship init bash)"']
            "#,
        )
        .unwrap();

        let starship = &raw.blocks["starship"];
        assert_eq!(
            starship.shell["fish"].source_lines,
            vec!["starship init fish | source".to_string()]
        );
        assert_eq!(
            starship.shell["bash"].source_lines,
            vec!["eval \"$(starship init bash)\"".to_string()]
        );
    }

    #[test]
    fn parses_yaml_root_shape() {
        let raw: RawConfig = serde_yaml::from_str(
            r"
blocks:
  simple:
    env:
      EDITOR: nvim
    alias:
      vim: nvim
    path:
      prepend:
        - ~/.local/bin
",
        )
        .unwrap();

        let simple = &raw.blocks["simple"];
        assert_eq!(simple.env["EDITOR"], EnvValue::from("nvim"));
        assert_eq!(simple.alias["vim"], "nvim");
        assert_eq!(simple.path.prepend, vec!["~/.local/bin"]);
    }

    #[test]
    fn parses_non_string_env_scalars_in_yaml() {
        let raw: RawConfig = serde_yaml::from_str(
            r"
blocks:
  simple:
    env:
      ENABLE_TRACE: true
      RETRIES: 3
    shell:
      bash:
        env:
          FEATURE_GATE: false
          DEPTH: 1
",
        )
        .unwrap();

        let simple = &raw.blocks["simple"];
        assert_eq!(simple.env["ENABLE_TRACE"], EnvValue::from(true));
        assert_eq!(simple.env["RETRIES"], EnvValue::from(3_i64));
        assert_eq!(
            simple.shell["bash"].env["FEATURE_GATE"],
            EnvValue::from(false)
        );
        assert_eq!(simple.shell["bash"].env["DEPTH"], EnvValue::from(1_i64));
    }

    #[test]
    fn parses_json_root_shape() {
        let raw: RawConfig = serde_json::from_str(
            r#"{
  "blocks": {
    "simple": {
      "env": {
        "EDITOR": "nvim"
      },
      "alias": {
        "vim": "nvim"
      },
      "path": {
        "prepend": ["~/.local/bin"]
      }
    }
  }
}"#,
        )
        .unwrap();

        let simple = &raw.blocks["simple"];
        assert_eq!(simple.env["EDITOR"], EnvValue::from("nvim"));
        assert_eq!(simple.alias["vim"], "nvim");
        assert_eq!(simple.path.prepend, vec!["~/.local/bin"]);
    }

    #[test]
    fn parses_non_string_env_scalars_in_json() {
        let raw: RawConfig = serde_json::from_str(
            r#"{
  "blocks": {
    "simple": {
      "env": {
        "ENABLE_TRACE": true,
        "RETRIES": 3
      },
      "shell": {
        "fish": {
          "env": {
            "FEATURE_GATE": false,
            "DEPTH": 1
          }
        }
      }
    }
  }
}"#,
        )
        .unwrap();

        let simple = &raw.blocks["simple"];
        assert_eq!(simple.env["ENABLE_TRACE"], EnvValue::from(true));
        assert_eq!(simple.env["RETRIES"], EnvValue::from(3_i64));
        assert_eq!(
            simple.shell["fish"].env["FEATURE_GATE"],
            EnvValue::from(false)
        );
        assert_eq!(simple.shell["fish"].env["DEPTH"], EnvValue::from(1_i64));
    }

    #[test]
    fn rejects_unsupported_env_scalar_types() {
        let err = toml::from_str::<RawConfig>(
            r"
            [blocks.demo.env]
            FRACTION = 1.5
            ",
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("a string, integer, boolean env value"),
            "unexpected parse error (expected EnvValue type hint): {msg}"
        );
    }

    #[test]
    fn rejects_non_integer_floats_in_json_env() {
        let err = serde_json::from_str::<RawConfig>(
            r#"{
  "blocks": {
    "demo": {
      "env": { "FRACTION": 1.5 }
    }
  }
}"#,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("a string, integer, boolean env value"),
            "unexpected parse error (expected EnvValue type hint): {msg}"
        );
    }

    #[test]
    fn parses_whole_number_floats_in_json_env_as_integers() {
        let raw: RawConfig = serde_json::from_str(
            r#"{
  "blocks": {
    "demo": {
      "env": { "RETRIES": 3.0 }
    }
  }
}"#,
        )
        .unwrap();

        assert_eq!(
            raw.blocks["demo"].env["RETRIES"],
            EnvValue::Integer("3".into())
        );
    }

    #[test]
    fn rejects_json_float_env_integers_with_non_safe_magnitude() {
        let err = serde_json::from_str::<RawConfig>(
            r#"{
  "blocks": {
    "demo": {
      "env": { "BIG": 1e20 }
    }
  }
}"#,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("a string, integer, boolean env value"),
            "unexpected parse error (expected EnvValue type hint): {msg}"
        );
    }

    #[test]
    fn rejects_json_float_env_at_unsafe_integer_boundary() {
        let err = serde_json::from_str::<RawConfig>(
            r#"{
  "blocks": {
    "demo": {
      "env": { "EDGE": 9007199254740992.0 }
    }
  }
}"#,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("a string, integer, boolean env value"),
            "unexpected parse error (expected EnvValue type hint): {msg}"
        );
    }

    #[test]
    fn parses_raw_env_table_in_yaml() {
        let raw: RawConfig = serde_yaml::from_str(
            r"
blocks:
  demo:
    env:
      LEAN_CTX_BIN:
        raw: $(command -v lean-ctx)
",
        )
        .unwrap();

        assert_eq!(
            raw.blocks["demo"].env["LEAN_CTX_BIN"],
            EnvValue::Raw("$(command -v lean-ctx)".into())
        );
    }

    #[test]
    fn parses_raw_env_table_in_json() {
        let raw: RawConfig = serde_json::from_str(
            r#"{
  "blocks": {
    "demo": {
      "env": {
        "LEAN_CTX_BIN": { "raw": "$(command -v lean-ctx)" }
      }
    }
  }
}"#,
        )
        .unwrap();

        assert_eq!(
            raw.blocks["demo"].env["LEAN_CTX_BIN"],
            EnvValue::Raw("$(command -v lean-ctx)".into())
        );
    }

    #[test]
    fn rejects_unknown_root_keys_in_toml() {
        let err = toml::from_str::<RawConfig>(
            r#"
            version = 1
            [blocks.demo]
            "#,
        )
        .unwrap_err();
        assert_unknown_field_error(err);
    }

    #[test]
    fn rejects_unknown_app_keys_in_toml() {
        let err = toml::from_str::<RawConfig>(
            r#"
            [blocks.demo]
            whehn = ["interactive"]
            "#,
        )
        .unwrap_err();
        assert_unknown_field_error(err);
    }

    #[test]
    fn rejects_unknown_path_keys_in_toml() {
        let err = toml::from_str::<RawConfig>(
            r#"
            [blocks.demo.path]
            preprend = ["~/bin"]
            "#,
        )
        .unwrap_err();
        assert_unknown_field_error(err);
    }

    #[test]
    fn rejects_unknown_shell_override_keys_in_toml() {
        let err = toml::from_str::<RawConfig>(
            r#"
            [blocks.demo.shell.fish]
            soruce_lines = ["echo hi"]
            "#,
        )
        .unwrap_err();
        assert_unknown_field_error(err);
    }

    #[test]
    fn rejects_unknown_keys_in_yaml() {
        let err = serde_yaml::from_str::<RawConfig>(
            r"
version: 1
blocks:
  demo: {}
",
        )
        .unwrap_err();
        assert_unknown_field_error(err);
    }

    #[test]
    fn rejects_unknown_keys_in_json() {
        let err = serde_json::from_str::<RawConfig>(
            r#"{
  "version": 1,
  "blocks": { "demo": {} }
}"#,
        )
        .unwrap_err();
        assert_unknown_field_error(err);
    }
}
