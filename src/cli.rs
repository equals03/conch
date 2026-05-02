//! CLI entry (`check`, `init`, `build`, `explain`, `complete`).

use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};

use crate::build::{
    resolve_build, resolve_build_with_details, resolve_init, resolve_init_with_details,
    HostFoldContext,
};
use crate::config::RawConfig;
use crate::error::ConchError;
use crate::explain::{render_resolution_for, ExplainMode, RenderOptions};
use crate::provider::{BashProvider, FishProvider};
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
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, uses `CONCH_CONFIG` when set, otherwise searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Target shell to validate. If omitted, conch validates both fish and bash.
        #[arg(value_enum)]
        shell: Option<ShellKind>,
        /// Explain what `check` would validate instead of returning success-only output.
        #[arg(long)]
        explain: bool,
        /// Control ANSI color in explain output.
        #[arg(long, value_enum, default_value = "auto")]
        color: ColorMode,
    },
    /// Generate shell-native init output and print it to stdout.
    Init {
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, uses `CONCH_CONFIG` when set, otherwise searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Target shell to generate init output for.
        #[arg(value_enum)]
        shell: ShellKind,
        /// Override OS label for `os:` predicate folding (e.g. `linux`, `macos`). Default: detected from the compiler target.
        #[arg(long)]
        os: Option<String>,
        /// Override hostname for `hostname:` predicate folding. Default: OS hostname.
        #[arg(long)]
        hostname: Option<String>,
        /// Explain what `init` would emit instead of printing shell output.
        #[arg(long)]
        explain: bool,
        /// Control ANSI color in explain output.
        #[arg(long, value_enum, default_value = "auto")]
        color: ColorMode,
    },
    /// Generate host-bound shell output with build-time folding of selected predicates.
    Build {
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, uses `CONCH_CONFIG` when set, otherwise searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Target shell to generate build output for.
        #[arg(value_enum)]
        shell: ShellKind,
        /// Override OS label for `os:` predicate folding (e.g. `linux`, `macos`). Default: detected from the compiler target.
        #[arg(long)]
        os: Option<String>,
        /// Override hostname for `hostname:` predicate folding. Default: OS hostname.
        #[arg(long)]
        hostname: Option<String>,
        /// Explain what `build` would emit instead of printing shell output.
        #[arg(long)]
        explain: bool,
        /// Control ANSI color in explain output.
        #[arg(long, value_enum, default_value = "auto")]
        color: ColorMode,
    },
    /// Explain ordered blocks, guards, and write ordering for a target shell.
    Explain {
        /// Path to config file (.toml, .yaml, .yml, or .json). If omitted, uses `CONCH_CONFIG` when set, otherwise searches XDG config locations.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Target shell to explain.
        #[arg(value_enum)]
        shell: ShellKind,
        /// Optional action semantics to explain. Defaults to `init`.
        #[arg(value_enum)]
        action: Option<ExplainAction>,
        /// Override OS label for `os:` folding when action is `init` or `build`.
        #[arg(long)]
        os: Option<String>,
        /// Override hostname for `hostname:` folding when action is `init` or `build`.
        #[arg(long)]
        hostname: Option<String>,
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
pub enum ExplainAction {
    Check,
    Init,
    Build,
}

impl ExplainAction {
    fn mode(self) -> ExplainMode {
        match self {
            Self::Check => ExplainMode::Check,
            Self::Init => ExplainMode::Init,
            Self::Build => ExplainMode::Build,
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
const CONCH_CONFIG_ENV: &str = "CONCH_CONFIG";

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
        Command::Check {
            config,
            shell,
            explain,
            color,
        } => {
            let raw = load_selected_config(config)?;
            if explain {
                print_check_explain(&raw, shell, color)?;
            } else {
                run_check(&raw, shell)?;
            }
            Ok(())
        }
        Command::Init {
            config,
            shell,
            os,
            hostname,
            explain,
            color,
        } => {
            let raw = load_selected_config(config)?;
            let host_ctx = host_fold_context(os, hostname);
            if explain {
                print_explain(&raw, shell, ExplainAction::Init, color, &host_ctx)?;
            } else {
                print_shell_output(&raw, shell, ExplainAction::Init, &host_ctx)?;
            }
            Ok(())
        }
        Command::Build {
            config,
            shell,
            os,
            hostname,
            explain,
            color,
        } => {
            let raw = load_selected_config(config)?;
            let host_ctx = host_fold_context(os, hostname);
            if explain {
                print_explain(&raw, shell, ExplainAction::Build, color, &host_ctx)?;
            } else {
                print_shell_output(&raw, shell, ExplainAction::Build, &host_ctx)?;
            }
            Ok(())
        }
        Command::Explain {
            config,
            shell,
            action,
            os,
            hostname,
            color,
        } => {
            let action = action.unwrap_or(ExplainAction::Init);
            validate_explain_host_flags(action, os.as_deref(), hostname.as_deref())?;
            let raw = load_selected_config(config)?;
            let host_ctx = host_fold_context(os, hostname);
            print_explain(&raw, shell, action, color, &host_ctx)?;
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

fn run_check(raw: &RawConfig, shell: Option<ShellKind>) -> Result<(), ConchError> {
    match shell {
        Some(shell) => {
            resolve(raw, shell.as_str())?;
        }
        None => {
            for shell in [ShellKind::Fish, ShellKind::Bash] {
                resolve(raw, shell.as_str())?;
            }
        }
    }
    Ok(())
}

fn host_fold_context(os: Option<String>, hostname: Option<String>) -> HostFoldContext {
    HostFoldContext {
        os: os
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty()),
        hostname: hostname
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}

fn validate_explain_host_flags(
    action: ExplainAction,
    os: Option<&str>,
    hostname: Option<&str>,
) -> Result<(), ConchError> {
    if matches!(action, ExplainAction::Check) && (os.is_some() || hostname.is_some()) {
        return Err(ConchError::Validation(
            "`conch explain ... check` does not use `--os` or `--hostname`; remove those flags or choose `init` / `build`".into(),
        ));
    }
    Ok(())
}

fn print_shell_output(
    raw: &RawConfig,
    shell: ShellKind,
    action: ExplainAction,
    host_ctx: &HostFoldContext,
) -> Result<(), ConchError> {
    let ir = match action {
        ExplainAction::Init => resolve_init(raw, shell.as_str(), host_ctx)?,
        ExplainAction::Build => resolve_build(raw, shell.as_str(), host_ctx)?,
        ExplainAction::Check => unreachable!("check does not emit shell output"),
    };
    let text = match shell {
        ShellKind::Fish => FishProvider.render_init(&ir, raw.init.guard.enabled),
        ShellKind::Bash => BashProvider.render_init(&ir, raw.init.guard.enabled),
    };
    print!("{text}");
    Ok(())
}

fn print_explain(
    raw: &RawConfig,
    shell: ShellKind,
    action: ExplainAction,
    color: ColorMode,
    host_ctx: &HostFoldContext,
) -> Result<(), ConchError> {
    let text = render_explain(raw, shell, action, color, host_ctx)?;
    print!("{text}");
    Ok(())
}

fn render_explain(
    raw: &RawConfig,
    shell: ShellKind,
    action: ExplainAction,
    color: ColorMode,
    host_ctx: &HostFoldContext,
) -> Result<String, ConchError> {
    let resolution = match action {
        ExplainAction::Check => resolve_with_details(raw, shell.as_str())?,
        ExplainAction::Init => resolve_init_with_details(raw, shell.as_str(), host_ctx)?,
        ExplainAction::Build => resolve_build_with_details(raw, shell.as_str(), host_ctx)?,
    };
    Ok(render_resolution_for(
        &resolution,
        RenderOptions {
            color: color.use_color(),
        },
        action.mode(),
    ))
}

fn print_check_explain(
    raw: &RawConfig,
    shell: Option<ShellKind>,
    color: ColorMode,
) -> Result<(), ConchError> {
    let host_ctx = HostFoldContext::default();
    let text = match shell {
        Some(shell) => render_explain(raw, shell, ExplainAction::Check, color, &host_ctx)?,
        None => {
            let fish =
                render_explain(raw, ShellKind::Fish, ExplainAction::Check, color, &host_ctx)?;
            let bash =
                render_explain(raw, ShellKind::Bash, ExplainAction::Check, color, &host_ctx)?;
            format!("{fish}\n\n{bash}")
        }
    };
    print!("{text}");
    Ok(())
}

fn load_selected_config(cli_config: Option<PathBuf>) -> Result<RawConfig, ConchError> {
    let path = match cli_config {
        Some(path) => path,
        None => match env_conch_config_path() {
            Some(path) => path,
            None => resolve_default_config_path()?,
        },
    };
    load_config(&path)
}

/// Config path from `CONCH_CONFIG` when set and non-empty; otherwise `None` so callers can fall back to XDG.
fn env_conch_config_path() -> Option<PathBuf> {
    let path = env::var_os(CONCH_CONFIG_ENV)?;
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

fn resolve_default_config_path() -> Result<PathBuf, ConchError> {
    let roots = xdg_config_roots()?;
    let mut candidates = Vec::new();
    for root in &roots {
        candidates.extend(config_candidates_in_root(root));
    }
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| ConchError::DefaultConfigNotFound(format_default_config_hint(&roots)))
}

fn xdg_config_roots() -> Result<Vec<PathBuf>, ConchError> {
    let mut roots = vec![xdg_config_home()?];
    roots.extend(xdg_config_dirs());
    Ok(roots)
}

fn xdg_config_home() -> Result<PathBuf, ConchError> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        if !path.is_empty() {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(ConchError::Validation(
                    "XDG_CONFIG_HOME must be an absolute path".into(),
                ));
            }
            return Ok(path);
        }
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

const DEFAULT_CONFIG_HINT_MAX_DIRS: usize = 8;

/// Short user-facing hint: lists XDG roots, not every extension permutation (avoids huge errors on long `XDG_CONFIG_DIRS`).
fn format_default_config_hint(roots: &[PathBuf]) -> String {
    let dirs = format_dir_list_for_hint(roots);
    format!(
        "Searched for conch.{{toml,yaml,yml,json}} or conch/config.* under {dirs}. Pass --config or set {CONCH_CONFIG_ENV}."
    )
}

fn format_dir_list_for_hint(roots: &[PathBuf]) -> String {
    if roots.is_empty() {
        return "(no config directories resolved)".to_string();
    }
    if roots.len() <= DEFAULT_CONFIG_HINT_MAX_DIRS {
        return roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let head: Vec<_> = roots
        .iter()
        .take(DEFAULT_CONFIG_HINT_MAX_DIRS)
        .map(|p| p.display().to_string())
        .collect();
    let n_more = roots.len() - DEFAULT_CONFIG_HINT_MAX_DIRS;
    format!("{}, … (+{n_more} more)", head.join(", "))
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
