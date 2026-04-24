//! Build-time predicate folding for host-bound shell output.
//!
//! `conch build` shares the same parse/order/conflict pipeline as `init`, then
//! evaluates host-bound predicates ahead of time and drops impossible blocks.

use std::env;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use indexmap::IndexMap;

use crate::config::RawConfig;
use crate::error::ConchError;
use crate::ir::{Action, Block, ResolvedIr};
use crate::predicate::{Predicate, PredicateAtom};
use crate::resolve::{
    resolve_with_details, BindingReport, BindingValue, BindingWrite, BlockReport, PathContribution,
    Resolution,
};

pub fn resolve_build(raw: &RawConfig, target_shell: &str) -> Result<ResolvedIr, ConchError> {
    Ok(resolve_build_with_details(raw, target_shell)?.ir)
}

pub fn resolve_build_with_details(
    raw: &RawConfig,
    target_shell: &str,
) -> Result<Resolution, ConchError> {
    let resolution = resolve_with_details(raw, target_shell)?;
    Ok(fold_resolution_for_build(resolution))
}

fn fold_resolution_for_build(resolution: Resolution) -> Resolution {
    let blocks: Vec<Block> = resolution
        .ir
        .blocks
        .iter()
        .filter_map(|block| fold_block_for_build(block, &resolution.target_shell))
        .collect();

    rebuild_resolution(&resolution.target_shell, blocks)
}

fn fold_block_for_build(block: &Block, target_shell: &str) -> Option<Block> {
    let when = fold_predicates_for_build(&block.when, target_shell)?;
    let requires = fold_predicates_for_build(&block.requires, target_shell)?;

    Some(Block {
        block_id: block.block_id.clone(),
        when,
        requires,
        actions: block.actions.clone(),
    })
}

fn fold_predicates_for_build(
    predicates: &[Predicate],
    target_shell: &str,
) -> Option<Vec<Predicate>> {
    let mut retained = Vec::new();

    for predicate in predicates {
        match eval_build_time_predicate(predicate, target_shell) {
            Some(true) => {}
            Some(false) => return None,
            None => retained.push(predicate.clone()),
        }
    }

    Some(retained)
}

fn eval_build_time_predicate(predicate: &Predicate, target_shell: &str) -> Option<bool> {
    let value = match &predicate.atom {
        PredicateAtom::Interactive
        | PredicateAtom::Login
        | PredicateAtom::EnvExists(_)
        | PredicateAtom::EnvEquals { .. } => return None,
        PredicateAtom::Shell(name) => name.eq_ignore_ascii_case(target_shell),
        PredicateAtom::Command(name) => command_lookup_build(name)?,
        PredicateAtom::File(path) => path_predicate_build(&expanded_path(path)?, false)?,
        PredicateAtom::Dir(path) => path_predicate_build(&expanded_path(path)?, true)?,
        PredicateAtom::Os(name) => {
            let kernel = kernel_uname_s()?;
            kernel.to_ascii_lowercase() == name.as_str()
        }
        PredicateAtom::Hostname(name) => {
            let host = host_name()?;
            host == *name
        }
    };

    Some(if predicate.negated { !value } else { value })
}

fn rebuild_resolution(target_shell: &str, blocks: Vec<Block>) -> Resolution {
    let mut env_writers: IndexMap<String, Vec<BindingWrite>> = IndexMap::new();
    let mut alias_writers: IndexMap<String, Vec<BindingWrite>> = IndexMap::new();
    let mut path_ops = Vec::new();
    let mut block_reports = Vec::new();

    for block in &blocks {
        let mut source_count = 0;
        let mut source_line_count = 0;

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
                Action::Source(_) => source_count += 1,
                Action::SourceLines { lines } => source_line_count += lines.len(),
            }
        }

        block_reports.push(BlockReport {
            block_id: block.block_id.clone(),
            when: block.when.iter().map(ToString::to_string).collect(),
            requires: block.requires.iter().map(ToString::to_string).collect(),
            guarded: !(block.when.is_empty() && block.requires.is_empty()),
            action_count: block.actions.len(),
            source_count,
            source_line_count,
        });
    }

    let block_order = blocks.iter().map(|block| block.block_id.clone()).collect();

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

fn kernel_uname_s() -> Option<String> {
    let output = Command::new("uname").arg("-s").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Some(home_dir()?.join(rest));
    }
    Some(PathBuf::from(value))
}

fn host_name() -> Option<String> {
    if let Some(value) = env::var_os("HOSTNAME") {
        let value = value.to_string_lossy().trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    if let Some(value) = env::var_os("COMPUTERNAME") {
        let value = value.to_string_lossy().trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }

    let output = Command::new("hostname").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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
        BlockConfigToml, EnvValue, RawConfig, SourceEntryFieldsToml, SourceEntryToml,
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

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
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

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert_eq!(resolution.ir.blocks[0].when.len(), 1);
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);
        assert_eq!(
            resolution.ir.blocks[0].when[0].to_string(),
            "interactive"
        );
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

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert_eq!(resolution.ir.blocks[0].requires.len(), 1);
    }

    #[test]
    fn build_keeps_file_predicate_when_home_is_empty_for_tilde_path() {
        let _env_lock = ENV_MUTEX.lock().unwrap();
        let _home_guard = EnvVarGuard::replace("HOME", Some(OsString::new()));

        let mut block = BlockConfigToml::default();
        block
            .requires
            .push("file:~/.conch-build-empty-home".into());
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
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

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
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

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(matches!(
            resolution.ir.blocks[0].actions[0],
            Action::Source(crate::config::SourceEntry::Command(_))
        ));
    }

    #[test]
    fn build_folds_os_predicate_using_uname_kernel_name() {
        let kernel = Command::new("uname")
            .arg("-s")
            .output()
            .expect("uname -s");
        let stdout = String::from_utf8_lossy(&kernel.stdout);
        let kernel = stdout.trim();
        assert!(!kernel.is_empty(), "uname -s returned empty output");
        let lowered = kernel.to_ascii_lowercase();

        let mut block = BlockConfigToml::default();
        block.when.push(format!("os:{lowered}"));
        block.alias.insert("vim".into(), "nvim".into());
        let raw = RawConfig {
            init: Default::default(),
            blocks: IndexMap::from([("demo".into(), block)]),
        };

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
        assert_eq!(resolution.ir.blocks.len(), 1);
        assert!(resolution.ir.blocks[0].when.is_empty());
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

        let resolution = resolve_build_with_details(&raw, "fish").unwrap();
        assert_eq!(resolution.block_order, vec!["kept"]);
        assert_eq!(resolution.block_reports.len(), 1);
        assert_eq!(resolution.block_reports[0].when, vec!["interactive"]);
        assert_eq!(resolution.env_bindings.len(), 1);
        assert!(resolution.alias_bindings.is_empty());
    }
}
