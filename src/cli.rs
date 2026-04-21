//! CLI entry (`check`, `init`, `explain`, `complete`).

use std::env;
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
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, conch searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Target shell to validate. If omitted, conch validates both fish and bash.
        #[arg(value_enum)]
        shell: Option<ShellKind>,
    },
    /// Generate shell-native init output and print it to stdout.
    Init {
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, conch searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Target shell to generate init output for.
        #[arg(value_enum)]
        shell: ShellKind,
    },
    /// Explain ordered blocks, guards, and write ordering for a target shell.
    Explain {
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, conch searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
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

const CONFIG_EXTENSIONS: [&str; 4] = ["toml", "yaml", "yml", "json"];
const CONFIG_SEARCH_STEMS: [&str; 2] = ["conch", "conch/config"];
const XDG_DEFAULT_CONFIG_DIRS: &str = "/etc/xdg";

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
            let raw = load_selected_config(config)?;
            match shell {
                Some(shell) => {
                    resolve(&raw, shell.as_str())?;
                }
                None => {
                    for shell in [ShellKind::Fish, ShellKind::Bash] {
                        resolve(&raw, shell.as_str())?;
                    }
                }
            }
            Ok(())
        }
        Command::Init { config, shell } => {
            let raw = load_selected_config(config)?;
            let ir = resolve(&raw, shell.as_str())?;
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
            let raw = load_selected_config(config)?;
            let resolution = resolve_with_details(&raw, shell.as_str())?;
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

fn load_selected_config(path: Option<PathBuf>) -> Result<RawConfig, ConchError> {
    let path = match path {
        Some(path) => path,
        None => resolve_default_config_path()?,
    };
    load_config(&path)
}

fn resolve_default_config_path() -> Result<PathBuf, ConchError> {
    let mut candidates = Vec::new();
    for root in xdg_config_roots()? {
        candidates.extend(config_candidates_in_root(&root));
    }
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| {
            ConchError::DefaultConfigNotFound(format_default_config_candidates(&candidates))
        })
}

fn xdg_config_roots() -> Result<Vec<PathBuf>, ConchError> {
    let mut roots = vec![xdg_config_home()?];
    roots.extend(xdg_config_dirs());
    Ok(roots)
}

fn xdg_config_home() -> Result<PathBuf, ConchError> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        if path.is_empty() {
            return Err(ConchError::Validation(
                "XDG_CONFIG_HOME must not be empty when set".into(),
            ));
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(ConchError::Validation(
                "XDG_CONFIG_HOME must be an absolute path".into(),
            ));
        }
        return Ok(path);
    }

    match env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home).join(".config")),
        _ => Err(ConchError::Validation(
            "could not resolve XDG config home; set XDG_CONFIG_HOME or HOME".into(),
        )),
    }
}

fn xdg_config_dirs() -> Vec<PathBuf> {
    let Some(paths) = env::var_os("XDG_CONFIG_DIRS") else {
        return vec![PathBuf::from(XDG_DEFAULT_CONFIG_DIRS)];
    };
    if paths.is_empty() {
        return vec![PathBuf::from(XDG_DEFAULT_CONFIG_DIRS)];
    }

    let dirs: Vec<PathBuf> = env::split_paths(&paths)
        .filter(|path| path.is_absolute())
        .collect();
    if dirs.is_empty() {
        vec![PathBuf::from(XDG_DEFAULT_CONFIG_DIRS)]
    } else {
        dirs
    }
}

fn config_candidates_in_root(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for stem in CONFIG_SEARCH_STEMS {
        for extension in CONFIG_EXTENSIONS {
            candidates.push(root.join(format!("{stem}.{extension}")));
        }
    }
    candidates
}

fn format_default_config_candidates(candidates: &[PathBuf]) -> String {
    candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
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
