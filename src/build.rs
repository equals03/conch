//! Build-time predicate folding for host-bound shell output.
//!
//! `conch build` shares the same parse/order/conflict pipeline as `init`, then
//! evaluates host-bound predicates ahead of time and drops impossible blocks.
//!
//! `conch init` applies a narrower fold: `shell:`, `os:`, and `hostname:` are resolved ahead
//! of time, so emitted scripts avoid impossible target-shell blocks and need not call `uname` /
//! `hostname`.
//!
//! Fold values default to [`std::env::consts::OS`] and [`hostname::get`]; use
//! [`HostFoldContext`] (or `conch init` / `build` `--os` / `--hostname`) to override.

use std::env;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::config::{Config, RawConfig};
use crate::error::ConchError;
use crate::graph::build_graph;
use crate::ir::{Action, Block, ResolvedIr};
use crate::predicate::{Predicate, PredicateAtom};
use crate::provider::subst::{parse_predicate_path_interp, InterpSegment};
use crate::resolve::{
    actions_for_block, parse_predicates_for, BindingReport, BindingValue, BindingWrite,
    BlockReport, PathContribution, Resolution,
};

/// Overrides for folding `os:` and `hostname:` predicates during `conch init` / `conch build`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostFoldContext {
    /// Lowercase OS label (e.g. `linux`, `macos`) for `os:` folding; `None` uses [`detect_os`].
    pub os: Option<String>,
    /// Hostname for `hostname:` folding; `None` uses [`detect_hostname`].
    pub hostname: Option<String>,
}

impl HostFoldContext {
    fn effective_os(&self) -> String {
        self.os
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(detect_os)
    }

    fn effective_hostname(&self) -> Option<String> {
        if let Some(ref h) = self.hostname {
            let t = h.trim();
            if t.is_empty() {
                return None;
            }
            return Some(t.to_string());
        }
        detect_hostname()
    }
}

/// OS string for predicate folding: [`std::env::consts::OS`] in ASCII lowercase.
pub fn detect_os() -> String {
    std::env::consts::OS.to_ascii_lowercase()
}

/// Hostname from the OS (`gethostname`), without running the `hostname` executable.
pub fn detect_hostname() -> Option<String> {
    let bytes = hostname::get().ok()?;
    let s = bytes.to_string_lossy().trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn os_predicate_matches(host_os: &str, config_value: &str) -> bool {
    let host_os = host_os.trim().to_ascii_lowercase();
    let pred = config_value.trim().to_ascii_lowercase();
    if host_os == pred {
        return true;
    }
    // Legacy configs used `uname -s` style `Darwin` → `os:darwin`; Rust reports `macos`.
    (host_os == "macos" && pred == "darwin") || (host_os == "darwin" && pred == "macos")
}

pub fn resolve_build(
    raw: &RawConfig,
    target_shell: &str,
    host: &HostFoldContext,
) -> Result<ResolvedIr, ConchError> {
    Ok(resolve_build_with_details(raw, target_shell, host)?.ir)
}

pub fn resolve_build_with_details(
    raw: &RawConfig,
    target_shell: &str,
    host: &HostFoldContext,
) -> Result<Resolution, ConchError> {
    resolve_folded_with_details(raw, target_shell, PredicateFoldMode::Full, host)
}

/// Like [`resolve_build`], but only folds `shell:`, `os:`, and `hostname:` for the selected
/// target shell and the host that runs `conch`.
pub fn resolve_init(
    raw: &RawConfig,
    target_shell: &str,
    host: &HostFoldContext,
) -> Result<ResolvedIr, ConchError> {
    Ok(resolve_init_with_details(raw, target_shell, host)?.ir)
}

pub fn resolve_init_with_details(
    raw: &RawConfig,
    target_shell: &str,
    host: &HostFoldContext,
) -> Result<Resolution, ConchError> {
    resolve_folded_with_details(raw, target_shell, PredicateFoldMode::Init, host)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PredicateFoldMode {
    /// Fold every predicate we can evaluate on this machine (full `conch build` semantics).
    Full,
    /// Fold `shell:`, `os:`, and `hostname:` so `conch init` output avoids impossible target
    /// shell blocks and shell calls to `uname` / `hostname`.
    Init,
}

fn resolve_folded_with_details(
    raw: &RawConfig,
    target_shell: &str,
    mode: PredicateFoldMode,
    host: &HostFoldContext,
) -> Result<Resolution, ConchError> {
    let config = Config::try_from(raw)?;
    let block_ids: Vec<String> = config.blocks.keys().cloned().collect();
    let graph = build_graph(&config, &block_ids)?;
    let order = graph.topo_order()?;

    let mut blocks = Vec::new();
    let mut block_order = Vec::new();
    let mut block_reports = Vec::new();

    for block_id in &order {
        let block_cfg = &config.blocks[block_id];
        let actions = actions_for_block(block_cfg, target_shell);
        let source_count = actions
            .iter()
            .filter(|action| matches!(action, Action::Source(_)))
            .count();
        let source_line_count = actions
            .iter()
            .filter_map(|action| match action {
                Action::SourceLines { lines } => Some(
                    lines
                        .iter()
                        .map(|line| crate::provider::verbatim_line_count(line))
                        .sum::<usize>(),
                ),
                _ => None,
            })
            .sum();
        let block = Block {
            block_id: block_id.clone(),
            when: parse_predicates_for(block_id, "when", &block_cfg.when)?,
            requires: parse_predicates_for(block_id, "requires", &block_cfg.requires)?,
            actions,
        };

        let Some(block) = fold_block(&block, target_shell, mode, host) else {
            continue;
        };

        block_order.push(block.block_id.clone());
        block_reports.push(BlockReport {
            block_id: block.block_id.clone(),
            when: block.when.iter().map(ToString::to_string).collect(),
            requires: block.requires.iter().map(ToString::to_string).collect(),
            guarded: !(block.when.is_empty() && block.requires.is_empty()),
            action_count: block.actions.len(),
            source_count,
            source_line_count,
        });

        if !block.actions.is_empty() {
            blocks.push(block);
        }
    }

    validate_folded_writers(&blocks, &graph, target_shell)?;
    Ok(rebuild_resolution(
        target_shell,
        block_order,
        block_reports,
        blocks,
    ))
}

fn fold_block(
    block: &Block,
    target_shell: &str,
    mode: PredicateFoldMode,
    host: &HostFoldContext,
) -> Option<Block> {
    let when = fold_predicates(&block.when, target_shell, mode, host)?;
    let requires = fold_predicates(&block.requires, target_shell, mode, host)?;

    Some(Block {
        block_id: block.block_id.clone(),
        when,
        requires,
        actions: block.actions.clone(),
    })
}

fn fold_predicates(
    predicates: &[Predicate],
    target_shell: &str,
    mode: PredicateFoldMode,
    host: &HostFoldContext,
) -> Option<Vec<Predicate>> {
    let mut retained = Vec::new();

    for predicate in predicates {
        match eval_build_time_predicate(predicate, target_shell, mode, host) {
            Some(true) => {}
            Some(false) => return None,
            None => retained.push(predicate.clone()),
        }
    }

    Some(retained)
}

fn eval_build_time_predicate(
    predicate: &Predicate,
    target_shell: &str,
    mode: PredicateFoldMode,
    host: &HostFoldContext,
) -> Option<bool> {
    let value = match &predicate.atom {
        PredicateAtom::Interactive
        | PredicateAtom::Login
        | PredicateAtom::EnvExists(_)
        | PredicateAtom::EnvEquals { .. } => return None,
        PredicateAtom::Shell(name)
            if matches!(mode, PredicateFoldMode::Full | PredicateFoldMode::Init) =>
        {
            name.eq_ignore_ascii_case(target_shell)
        }
        PredicateAtom::Shell(_) => return None,
        PredicateAtom::Command(name) if mode == PredicateFoldMode::Full => {
            command_lookup_build(name)?
        }
        PredicateAtom::Command(_) => return None,
        PredicateAtom::File(path) if mode == PredicateFoldMode::Full => {
            path_predicate_build(&expanded_path(path)?, false)?
        }
        PredicateAtom::File(_) => return None,
        PredicateAtom::Dir(path) if mode == PredicateFoldMode::Full => {
            path_predicate_build(&expanded_path(path)?, true)?
        }
        PredicateAtom::Dir(_) => return None,
        PredicateAtom::Os(name) => os_predicate_matches(&host.effective_os(), name),
        PredicateAtom::Hostname(name) => {
            let hostn = host.effective_hostname()?;
            hostn == *name
        }
    };

    Some(if predicate.negated { !value } else { value })
}

fn rebuild_resolution(
    target_shell: &str,
    block_order: Vec<String>,
    block_reports: Vec<BlockReport>,
    blocks: Vec<Block>,
) -> Resolution {
    let mut env_writers: IndexMap<String, Vec<BindingWrite>> = IndexMap::new();
    let mut alias_writers: IndexMap<String, Vec<BindingWrite>> = IndexMap::new();
    let mut path_ops = Vec::new();

    for block in &blocks {
        for action in &block.actions {
            match action {
                Action::SetEnv { key, value } => {
                    env_writers
                        .entry(key.clone())
                        .or_default()
                        .push(BindingWrite {
                            block_id: block.block_id.clone(),
                            value: BindingValue::Env(value.clone()),
                        });
                }
                Action::SetAlias { name, value } => {
                    alias_writers
                        .entry(name.clone())
                        .or_default()
                        .push(BindingWrite {
                            block_id: block.block_id.clone(),
                            value: BindingValue::Text(value.clone()),
                        });
                }
                Action::Path(op) => path_ops.push(PathContribution {
                    block_id: block.block_id.clone(),
                    op: op.clone(),
                }),
                Action::Source(_) | Action::SourceLines { .. } => {}
            }
        }
    }

    Resolution {
        target_shell: target_shell.to_string(),
        block_order,
        ir: ResolvedIr { blocks },
        block_reports,
        env_bindings: binding_reports(&env_writers),
        alias_bindings: binding_reports(&alias_writers),
        path_ops,
    }
}

fn binding_reports(writers: &IndexMap<String, Vec<BindingWrite>>) -> Vec<BindingReport> {
    writers
        .iter()
        .map(|(key, writes)| BindingReport {
            key: key.clone(),
            writers: writes.clone(),
        })
        .collect()
}

fn validate_folded_writers(
    blocks: &[Block],
    graph: &crate::graph::BlockGraph,
    target_shell: &str,
) -> Result<(), ConchError> {
    let mut env_writers: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut alias_writers: IndexMap<String, Vec<String>> = IndexMap::new();

    for block in blocks {
        for action in &block.actions {
            match action {
                Action::SetEnv { key, .. } => {
                    validate_folded_writer(
                        &mut env_writers,
                        key,
                        &block.block_id,
                        graph,
                        target_shell,
                        "env",
                    )?;
                }
                Action::SetAlias { name, .. } => {
                    validate_folded_writer(
                        &mut alias_writers,
                        name,
                        &block.block_id,
                        graph,
                        target_shell,
                        "alias",
                    )?;
                }
                Action::Path(_) | Action::Source(_) | Action::SourceLines { .. } => {}
            }
        }
    }

    Ok(())
}

fn validate_folded_writer(
    writers: &mut IndexMap<String, Vec<String>>,
    key: &str,
    block_id: &str,
    graph: &crate::graph::BlockGraph,
    target_shell: &str,
    kind: &str,
) -> Result<(), ConchError> {
    let entry = writers.entry(key.to_string()).or_default();
    for previous in entry.iter() {
        let ordered =
            graph.ordered_before(previous, block_id) || graph.ordered_before(block_id, previous);
        if !ordered {
            return Err(ConchError::MergeConflict(format!(
                "{kind} key `{key}` is written by blocks `{previous}` and `{block_id}` for shell `{target_shell}`, but the block graph does not order them. Add `before` or `after` to make the write order explicit."
            )));
        }
    }
    entry.push(block_id.to_string());
    Ok(())
}

fn command_lookup_build(name: &str) -> Option<bool> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return path_predicate_build(Path::new(name), false);
    }

    let path = env::var_os("PATH")?;
    let mut uncertain = false;
    for dir in env::split_paths(&path) {
        match path_predicate_build(&dir.join(name), false) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => uncertain = true,
        }
    }
    if uncertain {
        None
    } else {
        Some(false)
    }
}

fn path_predicate_build(path: &Path, expect_dir: bool) -> Option<bool> {
    match std::fs::metadata(path) {
        Ok(meta) => Some(if expect_dir {
            meta.is_dir()
        } else {
            meta.is_file()
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

fn home_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    if home.is_empty() {
        None
    } else {
        Some(PathBuf::from(home))
    }
}

fn expanded_path(value: &str) -> Option<PathBuf> {
    let segments = parse_predicate_path_interp(value).ok()?;
    let home = if segments
        .iter()
        .any(|segment| matches!(segment, InterpSegment::Home))
    {
        Some(home_dir()?.to_string_lossy().into_owned())
    } else {
        None
    };

    let mut rendered = String::new();
    for segment in segments {
        match segment {
            InterpSegment::Lit(text) => rendered.push_str(&text),
            InterpSegment::Home => rendered.push_str(home.as_ref().expect("home checked above")),
            InterpSegment::Env(_) => return None,
        }
    }

    Some(PathBuf::from(rendered))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use indexmap::IndexMap;

    use super::*;
    use crate::config::{
        BlockConfigToml, EnvValue, RawConfig, ShellOverridesToml, SourceEntryFieldsToml,
        SourceEntryToml,
    };

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn replace(key: &'static str, value: Option<OsString>) -> Self {
            let previous = env::var_os(key);
            match &value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            Self::replace(key, None)
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "conch-build-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn build_drops_blocks_with_false_shell_predicates() {
        let mut block = BlockConfigToml::default();
        block.when.push("shell:bash".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert!(resolution.ir.blocks.is_empty());
        assert!(resolution.block_order.is_empty());
    }

    #[test]
    fn build_keeps_runtime_only_predicates() {
        let mut block = BlockConfigToml::default();
        block.when.push("interactive".into());
        block.requires.push("env:EDITOR".into());
        block.requires.push("shell:fish".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert_eq!(resolution.ir.blocks[0].when.len(), 1);
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);
        assert_eq!(resolution.ir.blocks[0].when[0].to_string(), "interactive");
        assert_eq!(
            resolution.ir.blocks[0].requires[0].to_string(),
            "env:EDITOR"
        );
    }

    #[test]
    fn build_keeps_file_predicate_when_tilde_path_cannot_expand() {
        let _env_lock = ENV_MUTEX.lock().unwrap();
        let _home_guard = EnvVarGuard::unset("HOME");

        let mut block = BlockConfigToml::default();
        block
            .requires
            .push("file:~/.conch-build-missing-home".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);
    }

    #[test]
    fn build_keeps_file_predicate_when_home_is_empty_for_tilde_path() {
        let _env_lock = ENV_MUTEX.lock().unwrap();
        let _home_guard = EnvVarGuard::replace("HOME", Some(OsString::new()));

        let mut block = BlockConfigToml::default();
        block.requires.push("file:~/.conch-build-empty-home".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);
    }

    #[test]
    fn build_evaluates_file_dir_and_negated_command_predicates() {
        let root = temp_path("predicates");
        let dir = root.join("cfg");
        let file = dir.join("init.lua");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&file, "set number").unwrap();
        let missing_cmd = root.join("missing-cmd");

        let mut block = BlockConfigToml::default();
        block.requires.push(format!("file:{}", file.display()));
        block.requires.push(format!("dir:{}", dir.display()));
        block
            .requires
            .push(format!("!command:{}", missing_cmd.display()));
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].requires.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_preserves_structured_source_actions() {
        let mut block = BlockConfigToml::default();
        block.when.push("shell:fish".into());
        block
            .source
            .push(SourceEntryToml::Structured(SourceEntryFieldsToml {
                file: None,
                command: Some(vec!["starship".into(), "init".into(), "{shell}".into()]),
            }));
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("starship".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(matches!(
            resolution.ir.blocks[0].actions[0],
            Action::Source(crate::config::SourceEntry::Command(_))
        ));
    }

    #[test]
    fn build_folds_os_predicate_for_current_target() {
        let os = detect_os();

        let mut block = BlockConfigToml::default();
        block.when.push(format!("os:{os}"));
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].when.is_empty());
    }

    #[test]
    fn build_drops_block_when_os_override_mismatches_predicate() {
        let mut block = BlockConfigToml::default();
        block.when.push("os:linux".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };
        let host = HostFoldContext {
            os: Some("freebsd".into()),
            hostname: None,
        };
        let resolution = resolve_build_with_details(&raw, "fish", &host).unwrap();
        assert!(resolution.ir.blocks.is_empty());
    }

    #[test]
    fn build_folds_os_darwin_when_effective_os_is_macos() {
        let mut block = BlockConfigToml::default();
        block.when.push("os:darwin".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };
        let host = HostFoldContext {
            os: Some("macos".into()),
            hostname: None,
        };
        let resolution = resolve_build_with_details(&raw, "fish", &host).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].when.is_empty());
    }

    #[test]
    fn init_folds_os_predicate_but_keeps_file_predicate() {
        let root = temp_path("init-os-file");
        let file = root.join("marker");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, "").unwrap();

        let os = detect_os();

        let mut block = BlockConfigToml::default();
        block.when.push(format!("os:{os}"));
        block.requires.push(format!("file:{}", file.display()));
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_init_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].when.is_empty());
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn init_drops_false_shell_predicates() {
        let mut block = BlockConfigToml::default();
        block.when.push("shell:bash".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_init_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert!(resolution.ir.blocks.is_empty());
    }

    #[test]
    fn init_folds_true_shell_predicates() {
        let mut block = BlockConfigToml::default();
        block.when.push("shell:fish".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_init_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].when.is_empty());
    }

    #[test]
    fn init_drops_false_shell_blocks_before_conflict_validation() {
        let mut fish = BlockConfigToml::default();
        fish.when.push("shell:fish".into());
        fish.alias.insert("vim".into(), "nvim".into());

        let mut bash = BlockConfigToml::default();
        bash.when.push("shell:bash".into());
        bash.alias.insert("vim".into(), "hx".into());

        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("fish_only".into(), fish), ("bash_only".into(), bash)]),
        };

        let resolution =
            resolve_init_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.block_order, vec!["fish_only"]);
        assert_eq!(resolution.alias_bindings.len(), 1);
        assert_eq!(resolution.alias_bindings[0].key, "vim");
        assert_eq!(resolution.alias_bindings[0].writers.len(), 1);
        assert_eq!(
            resolution.alias_bindings[0].writers[0].block_id,
            "fish_only"
        );
    }

    #[test]
    fn folded_resolution_validates_predicates_even_when_block_has_no_actions() {
        let mut block = BlockConfigToml::default();
        block.when.push("shell".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("broken".into(), block)]),
        };

        let err = resolve_init_with_details(&raw, "fish", &HostFoldContext::default()).unwrap_err();
        assert!(matches!(err, ConchError::PredicateParse(_)));
        assert!(err
            .to_string()
            .contains("block `broken` has invalid `when` predicate"));
    }

    #[test]
    fn folded_resolution_keeps_actionless_blocks_in_reports() {
        let mut block = BlockConfigToml::default();
        block.when.push("shell:fish".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("empty".into(), block)]),
        };

        let resolution =
            resolve_init_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert!(resolution.ir.blocks.is_empty());
        assert_eq!(resolution.block_order, vec!["empty"]);
        assert_eq!(resolution.block_reports.len(), 1);
        assert_eq!(resolution.block_reports[0].block_id, "empty");
        assert_eq!(resolution.block_reports[0].action_count, 0);
        assert!(!resolution.block_reports[0].guarded);
    }

    #[test]
    fn build_rebuilds_reports_from_folded_blocks() {
        let mut kept = BlockConfigToml::default();
        kept.when.push("interactive".into());
        kept.requires.push("shell:fish".into());
        kept.env
            .insert("EDITOR".into(), EnvValue::String("nvim".into()));

        let mut dropped = BlockConfigToml::default();
        dropped.when.push("shell:bash".into());
        dropped.alias.insert("vim".into(), "hx".into());

        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("kept".into(), kept), ("dropped".into(), dropped)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.block_order, vec!["kept"]);
        assert_eq!(resolution.block_reports.len(), 1);
        assert_eq!(resolution.block_reports[0].when, vec!["interactive"]);
        assert_eq!(resolution.env_bindings.len(), 1);
        assert!(resolution.alias_bindings.is_empty());
    }

    #[test]
    fn build_counts_multiline_source_lines_in_reports() {
        let mut kept = BlockConfigToml::default();
        kept.shell.insert(
            "fish".into(),
            ShellOverridesToml {
                source_lines: vec!["echo a\necho b\n".into()],
                ..Default::default()
            },
        );

        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("kept".into(), kept)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.block_reports[0].source_line_count, 2);
    }

    #[test]
    fn build_keeps_file_predicates_with_env_interpolation() {
        let _env_lock = ENV_MUTEX.lock().unwrap();
        let temp_home = temp_path("home");
        let config_dir = temp_home.join(".config");
        let file = config_dir.join("nvim");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(&file, "set number").unwrap();
        let _home_guard = EnvVarGuard::replace("HOME", Some(temp_home.clone().into_os_string()));

        let mut block = BlockConfigToml::default();
        block.requires.push("file:${env:HOME}/.config/nvim".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);

        let _ = fs::remove_dir_all(&temp_home);
    }

    #[test]
    fn build_still_folds_absolute_file_predicates_without_home() {
        let _env_lock = ENV_MUTEX.lock().unwrap();
        let temp_root = temp_path("abs");
        let file = temp_root.join("demo.conf");
        fs::create_dir_all(&temp_root).unwrap();
        fs::write(&file, "demo").unwrap();
        let _home_guard = EnvVarGuard::unset("HOME");

        let mut block = BlockConfigToml::default();
        block.requires.push(format!("file:{}", file.display()));
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution =
            resolve_build_with_details(&raw, "fish", &HostFoldContext::default()).unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].requires.is_empty());

        let _ = fs::remove_dir_all(&temp_root);
    }
}
