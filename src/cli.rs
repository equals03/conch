//! CLI entry (`check`, `build`, `explain`, `complete`).

use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};

use crate::config::RawConfig;
use crate::error::ConchError;
use crate::explain::{render_resolution, RenderOptions};
use crate::provider::{BashProvider, FishProvider, ShellProvider};
use crate::resolve::{resolve, resolve_with_details};

/// Declarative shell-configuration compiler.
#[derive(Parser, Debug)]
#[command(name = "conch", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Parse config, validate schema, and report graph/conflict errors.
    Check {
        /// Path to config file (.toml, .yaml, .yml, or .json)
        #[arg(long, default_value = "conch.toml")]
        config: PathBuf,
        /// Target shell to validate. If omitted, conch validates both fish and bash.
        #[arg(value_enum)]
        shell: Option<ShellKind>,
    },
    /// Compile config and print shell-native output to stdout.
    Build {
        /// Path to config file (.toml, .yaml, .yml, or .json)
        #[arg(long, default_value = "conch.toml")]
        config: PathBuf,
        /// Target shell to compile for.
        #[arg(value_enum)]
        shell: ShellKind,
    },
    /// Explain ordered blocks, guards, and write ordering for a target shell.
    Explain {
        /// Path to config file (.toml, .yaml, .yml, or .json)
        #[arg(long, default_value = "conch.toml")]
        config: PathBuf,
        /// Target shell to explain.
        #[arg(value_enum)]
        shell: ShellKind,
        /// Control ANSI color in explain output.
        #[arg(long, value_enum, default_value = "auto")]
        color: ColorMode,
    },
    /// Print shell tab-completion definitions for `conch` to stdout.
    Complete {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ShellKind {
    Fish,
    Bash,
}

impl ShellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::Bash => "bash",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub fn use_color(self) -> bool {
        match self {
            Self::Auto => io::stdout().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum ConfigFormat {
    Toml,
    Yaml,
    Json,
}

impl ConfigFormat {
    fn from_path(path: &Path) -> Result<Self, ConchError> {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("toml") => Ok(Self::Toml),
            Some("yaml" | "yml") => Ok(Self::Yaml),
            Some("json") => Ok(Self::Json),
            _ => Err(ConchError::Validation(format!(
                "unsupported config format for `{}`; use .toml, .yaml, .yml, or .json",
                path.display()
            ))),
        }
    }

    fn parse(self, text: &str) -> Result<RawConfig, ConchError> {
        match self {
            Self::Toml => Ok(toml::from_str(text)?),
            Self::Yaml => Ok(serde_yaml::from_str(text)?),
            Self::Json => Ok(serde_json::from_str(text)?),
        }
    }
}

pub fn run() -> Result<(), ConchError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { config, shell } => {
            let raw = load_config(&config)?;
            match shell {
                Some(shell) => {
                    resolve(&raw, Some(shell.as_str()))?;
                }
                None => {
                    for shell in [ShellKind::Fish, ShellKind::Bash] {
                        resolve(&raw, Some(shell.as_str()))?;
                    }
                }
            }
            Ok(())
        }
        Command::Build { config, shell } => {
            let raw = load_config(&config)?;
            let ir = resolve(&raw, Some(shell.as_str()))?;
            let text = match shell {
                ShellKind::Fish => FishProvider.render(&ir),
                ShellKind::Bash => BashProvider.render(&ir),
            };
            print!("{text}");
            Ok(())
        }
        Command::Explain {
            config,
            shell,
            color,
        } => {
            let raw = load_config(&config)?;
            let resolution = resolve_with_details(&raw, Some(shell.as_str()))?;
            let text = render_resolution(
                &resolution,
                RenderOptions {
                    color: color.use_color(),
                },
            );
            print!("{text}");
            Ok(())
        }
        Command::Complete { shell } => {
            let mut cmd = Cli::command();
            let bin = cmd.get_name().to_owned();
            generate(shell, &mut cmd, bin, &mut io::stdout().lock());
            Ok(())
        }
    }
}

fn load_config(path: &Path) -> Result<RawConfig, ConchError> {
    if !path.exists() {
        return Err(ConchError::ConfigNotFound(path.to_path_buf()));
    }
    if !path.is_file() {
        return Err(ConchError::Validation(format!(
            "config path is not a file: {}",
            path.display()
        )));
    }
    let s = fs::read_to_string(path)?;
    let format = ConfigFormat::from_path(path)?;
    format.parse(&s)
}
