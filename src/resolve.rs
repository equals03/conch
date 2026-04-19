//! Graph ordering, conflict analysis, and shell-targeted resolution.
//!
//! Semantics:
//! - `conch` does not evaluate predicates against the host during build/check/explain
//! - all blocks are ordered statically through the block graph
//! - providers render `when` / `requires` as shell-native guards
//! - env and alias conflicts are checked conservatively across all blocks
//! - if two blocks write the same env/alias key and the graph does not order them,
//!   resolution fails even if their runtime predicates might differ

use indexmap::IndexMap;

use crate::config::{BlockConfig, Config, EnvValue, PathSpec, RawConfig, ShellOverride};
use crate::error::ConchError;
use crate::graph::build_graph;
use crate::ir::{Action, Block, PathOp, ResolvedIr};
use crate::predicate::{parse_predicates, Predicate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReport {
    pub block_id: String,
    pub when: Vec<String>,
    pub requires: Vec<String>,
    pub guarded: bool,
    pub action_count: usize,
    pub source_line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingValue {
    Env(EnvValue),
    Text(String),
}

impl BindingValue {
    pub fn describe(&self) -> String {
        match self {
            Self::Env(value) => value.describe(),
            Self::Text(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingWrite {
    pub block_id: String,
    pub value: BindingValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingReport {
    pub key: String,
    pub writers: Vec<BindingWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathContribution {
    pub block_id: String,
    pub op: PathOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub target_shell: Option<String>,
    pub block_order: Vec<String>,
    pub ir: ResolvedIr,
    pub block_reports: Vec<BlockReport>,
    pub env_bindings: Vec<BindingReport>,
    pub alias_bindings: Vec<BindingReport>,
    pub path_ops: Vec<PathContribution>,
}

pub fn resolve(raw: &RawConfig, target_shell: Option<&str>) -> Result<ResolvedIr, ConchError> {
    Ok(resolve_with_details(raw, target_shell)?.ir)
}

pub fn resolve_with_details(
    raw: &RawConfig,
    target_shell: Option<&str>,
) -> Result<Resolution, ConchError> {
    let config = Config::try_from(raw)?;

    let block_ids: Vec<String> = config.blocks.keys().cloned().collect();
    let graph = build_graph(&config, &block_ids)?;
    let order = graph.topo_order()?;
    let merge = build_blocks_and_reports(&config, &graph, &order, target_shell)?;

    Ok(Resolution {
        target_shell: target_shell.map(str::to_string),
        block_order: order,
        ir: merge.ir,
        block_reports: merge.block_reports,
        env_bindings: merge.env_bindings,
        alias_bindings: merge.alias_bindings,
        path_ops: merge.path_ops,
    })
}

struct MergeOutcome {
    ir: ResolvedIr,
    block_reports: Vec<BlockReport>,
    env_bindings: Vec<BindingReport>,
    alias_bindings: Vec<BindingReport>,
    path_ops: Vec<PathContribution>,
}

fn build_blocks_and_reports(
    config: &Config,
    graph: &crate::graph::BlockGraph,
    order: &[String],
    target_shell: Option<&str>,
) -> Result<MergeOutcome, ConchError> {
    let shell = target_shell.unwrap_or_default();
    let mut ir = ResolvedIr::default();
    let mut block_reports = Vec::new();
    let mut env_writers: IndexMap<String, Vec<BindingWrite>> = IndexMap::new();
    let mut alias_writers: IndexMap<String, Vec<BindingWrite>> = IndexMap::new();
    let mut path_ops = Vec::new();

    for block_id in order {
        let block_cfg = &config.blocks[block_id];
        let effective = block_for_shell(block_cfg, shell);
        let when = parse_predicates_for(block_id, "when", &block_cfg.when)?;
        let requires = parse_predicates_for(block_id, "requires", &block_cfg.requires)?;
        let actions = effective_actions(&effective);

        block_reports.push(BlockReport {
            block_id: block_id.clone(),
            when: block_cfg.when.clone(),
            requires: block_cfg.requires.clone(),
            guarded: !(block_cfg.when.is_empty() && block_cfg.requires.is_empty()),
            action_count: actions.len(),
            source_line_count: effective.source_lines.len(),
        });

        validate_writers(
            &mut env_writers,
            &effective.env,
            block_id,
            graph,
            shell,
            "env",
            |value| BindingValue::Env(value.clone()),
        )?;
        validate_writers(
            &mut alias_writers,
            &effective.alias,
            block_id,
            graph,
            shell,
            "alias",
            |value| BindingValue::Text(value.clone()),
        )?;

        path_ops.extend(path_actions(block_id, &effective.path));

        if !actions.is_empty() {
            ir.push_block(Block {
                block_id: block_id.clone(),
                when,
                requires,
                actions,
            });
        }
    }

    Ok(MergeOutcome {
        ir,
        block_reports,
        env_bindings: binding_reports(&env_writers),
        alias_bindings: binding_reports(&alias_writers),
        path_ops,
    })
}

fn parse_predicates_for(
    block_id: &str,
    field: &str,
    values: &[String],
) -> Result<Vec<Predicate>, ConchError> {
    parse_predicates(values).map_err(|err| match err {
        ConchError::PredicateParse(message) => ConchError::PredicateParse(format!(
            "block `{block_id}` has invalid `{field}` predicate {message}"
        )),
        other => other,
    })
}

fn validate_writers<T, F>(
    writers: &mut IndexMap<String, Vec<BindingWrite>>,
    incoming: &IndexMap<String, T>,
    block_id: &str,
    graph: &crate::graph::BlockGraph,
    shell: &str,
    kind: &str,
    bind_value: F,
) -> Result<(), ConchError>
where
    F: Fn(&T) -> BindingValue,
{
    for (key, value) in incoming {
        let entry = writers.entry(key.clone()).or_default();
        for previous in entry.iter() {
            let ordered = graph.ordered_before(&previous.block_id, block_id)
                || graph.ordered_before(block_id, &previous.block_id);
            if !ordered {
                let shell_context = if shell.is_empty() {
                    "for the current target".to_string()
                } else {
                    format!("for shell `{shell}`")
                };
                return Err(ConchError::MergeConflict(format!(
                    "{kind} key `{key}` is written by blocks `{}` and `{block_id}` {shell_context}, but the block graph does not order them. Add `before` or `after` to make the write order explicit.",
                    previous.block_id
                )));
            }
        }
        entry.push(BindingWrite {
            block_id: block_id.to_string(),
            value: bind_value(value),
        });
    }
    Ok(())
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

fn effective_actions(effective: &EffectiveBlock) -> Vec<Action> {
    let mut actions = Vec::new();
    actions.extend(effective.env.iter().map(|(key, value)| Action::SetEnv {
        key: key.clone(),
        value: value.clone(),
    }));
    actions.extend(
        effective
            .alias
            .iter()
            .map(|(name, value)| Action::SetAlias {
                name: name.clone(),
                value: value.clone(),
            }),
    );
    actions.extend(
        effective
            .path
            .prepend
            .iter()
            .cloned()
            .map(PathOp::Prepend)
            .map(Action::Path),
    );
    actions.extend(
        effective
            .path
            .append
            .iter()
            .cloned()
            .map(PathOp::Append)
            .map(Action::Path),
    );
    actions.extend(
        effective
            .path
            .move_front
            .iter()
            .cloned()
            .map(PathOp::MoveFront)
            .map(Action::Path),
    );
    actions.extend(
        effective
            .path
            .move_back
            .iter()
            .cloned()
            .map(PathOp::MoveBack)
            .map(Action::Path),
    );
    if !effective.source_lines.is_empty() {
        actions.push(Action::SourceLines {
            lines: effective.source_lines.clone(),
        });
    }
    actions
}

fn path_actions(block_id: &str, path: &PathSpec) -> Vec<PathContribution> {
    let mut actions = Vec::new();
    actions.extend(
        path.prepend
            .iter()
            .cloned()
            .map(|segment| PathContribution {
                block_id: block_id.to_string(),
                op: PathOp::Prepend(segment),
            }),
    );
    actions.extend(path.append.iter().cloned().map(|segment| PathContribution {
        block_id: block_id.to_string(),
        op: PathOp::Append(segment),
    }));
    actions.extend(
        path.move_front
            .iter()
            .cloned()
            .map(|segment| PathContribution {
                block_id: block_id.to_string(),
                op: PathOp::MoveFront(segment),
            }),
    );
    actions.extend(
        path.move_back
            .iter()
            .cloned()
            .map(|segment| PathContribution {
                block_id: block_id.to_string(),
                op: PathOp::MoveBack(segment),
            }),
    );
    actions
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EffectiveBlock {
    env: IndexMap<String, EnvValue>,
    alias: IndexMap<String, String>,
    path: PathSpec,
    source_lines: Vec<String>,
}

fn block_for_shell(block: &BlockConfig, shell: &str) -> EffectiveBlock {
    let mut effective = EffectiveBlock {
        env: block.env.clone(),
        alias: block.alias.clone(),
        path: block.path.clone(),
        ..Default::default()
    };

    if let Some(override_cfg) = block.shell.get(shell) {
        merge_override(&mut effective, override_cfg);
    }

    effective
}

fn merge_override(effective: &mut EffectiveBlock, override_cfg: &ShellOverride) {
    for (key, value) in &override_cfg.env {
        effective.env.insert(key.clone(), value.clone());
    }
    for (key, value) in &override_cfg.alias {
        effective.alias.insert(key.clone(), value.clone());
    }
    effective
        .path
        .prepend
        .extend(override_cfg.path.prepend.clone());
    effective
        .path
        .append
        .extend(override_cfg.path.append.clone());
    effective
        .path
        .move_front
        .extend(override_cfg.path.move_front.clone());
    effective
        .path
        .move_back
        .extend(override_cfg.path.move_back.clone());
    effective
        .source_lines
        .extend(override_cfg.source_lines.iter().cloned());
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::config::{BlockConfigToml, RawConfig, ShellOverridesToml};

    fn sample_block() -> BlockConfigToml {
        BlockConfigToml::default()
    }

    #[test]
    fn builds_ordered_guarded_blocks() {
        let mut base = sample_block();
        base.env.insert("EDITOR".into(), "vim".into());
        base.before.push("nvim".into());

        let mut nvim = sample_block();
        nvim.when.push("interactive".into());
        nvim.env.insert("EDITOR".into(), "nvim".into());
        nvim.after.push("base".into());

        let raw = RawConfig {
            blocks: IndexMap::from([("base".into(), base), ("nvim".into(), nvim)]),
        };

        let resolution = resolve_with_details(&raw, Some("fish")).unwrap();
        assert_eq!(resolution.block_order, vec!["base", "nvim"]);
        assert_eq!(resolution.ir.blocks.len(), 2);
        assert_eq!(resolution.ir.blocks[1].block_id, "nvim");
        assert_eq!(resolution.ir.blocks[1].when.len(), 1);
    }

    #[test]
    fn reports_unordered_conflicts_with_shell_context() {
        let mut a = sample_block();
        a.alias.insert("vim".into(), "nvim".into());

        let mut b = sample_block();
        b.alias.insert("vim".into(), "hx".into());

        let raw = RawConfig {
            blocks: IndexMap::from([("a".into(), a), ("b".into(), b)]),
        };

        let err = resolve(&raw, Some("fish")).unwrap_err();
        assert!(matches!(err, ConchError::MergeConflict(_)));
        assert!(err.to_string().contains("for shell `fish`"));
        assert!(err.to_string().contains("Add `before` or `after`"));
    }

    #[test]
    fn applies_shell_overrides() {
        let mut cfg = sample_block();
        cfg.alias.insert("vim".into(), "nvim".into());
        cfg.shell.insert(
            "fish".into(),
            ShellOverridesToml {
                alias: IndexMap::from([("v".into(), "nvim".into())]),
                ..Default::default()
            },
        );

        let raw = RawConfig {
            blocks: IndexMap::from([("nvim".into(), cfg)]),
        };

        let ir = resolve(&raw, Some("fish")).unwrap();
        assert_eq!(ir.blocks[0].actions.len(), 2);
    }

    #[test]
    fn includes_guarded_and_unguarded_blocks_in_reports() {
        let mut guarded = sample_block();
        guarded.when.push("interactive".into());
        guarded.alias.insert("cat".into(), "bat".into());

        let mut plain = sample_block();
        plain.alias.insert("vim".into(), "nvim".into());

        let raw = RawConfig {
            blocks: IndexMap::from([("guarded".into(), guarded), ("plain".into(), plain)]),
        };

        let resolution = resolve_with_details(&raw, Some("fish")).unwrap();
        assert_eq!(resolution.block_reports.len(), 2);
        assert!(resolution.block_reports.iter().any(|r| r.guarded));
        assert!(resolution.block_reports.iter().any(|r| !r.guarded));
    }

    #[test]
    fn reports_invalid_predicates_with_block_and_field_context() {
        let mut broken = sample_block();
        broken.when.push("shell".into());

        let raw = RawConfig {
            blocks: IndexMap::from([("broken".into(), broken)]),
        };

        let err = resolve(&raw, Some("fish")).unwrap_err();
        assert!(matches!(err, ConchError::PredicateParse(_)));
        assert!(err
            .to_string()
            .contains("block `broken` has invalid `when` predicate"));
    }

    #[test]
    fn reports_shell_specific_conflicts_only_for_the_target_shell() {
        let mut base = sample_block();
        base.alias.insert("v".into(), "vim".into());

        let mut fish_only = sample_block();
        fish_only.shell.insert(
            "fish".into(),
            ShellOverridesToml {
                alias: IndexMap::from([("v".into(), "nvim".into())]),
                ..Default::default()
            },
        );

        let raw = RawConfig {
            blocks: IndexMap::from([("base".into(), base), ("fish_only".into(), fish_only)]),
        };

        let fish_err = resolve(&raw, Some("fish")).unwrap_err();
        assert!(matches!(fish_err, ConchError::MergeConflict(_)));

        let bash_ir = resolve(&raw, Some("bash")).unwrap();
        assert_eq!(bash_ir.blocks.len(), 1);
    }

    #[test]
    fn ignores_unknown_shell_override_sections_for_other_targets() {
        let mut cfg = sample_block();
        cfg.alias.insert("cat".into(), "bat".into());
        cfg.shell.insert(
            "zsh".into(),
            ShellOverridesToml {
                alias: IndexMap::from([("c".into(), "bat --color=always".into())]),
                ..Default::default()
            },
        );

        let raw = RawConfig {
            blocks: IndexMap::from([("bat".into(), cfg)]),
        };

        let fish_ir = resolve(&raw, Some("fish")).unwrap();
        assert_eq!(fish_ir.blocks[0].actions.len(), 1);
    }

    #[test]
    fn tracks_path_operation_sources() {
        let mut base = sample_block();
        base.path.prepend.push("~/.local/bin".into());
        base.path.move_front.push("~/go/bin".into());

        let raw = RawConfig {
            blocks: IndexMap::from([("base".into(), base)]),
        };

        let resolution = resolve_with_details(&raw, Some("fish")).unwrap();
        assert_eq!(resolution.path_ops.len(), 2);
        assert_eq!(resolution.path_ops[0].block_id, "base");
        assert_eq!(resolution.path_ops[1].block_id, "base");
    }

    #[test]
    fn merges_source_lines_for_target_shell() {
        let mut starship = sample_block();
        starship.when.push("interactive".into());
        starship.requires.push("command:starship".into());
        starship.shell.insert(
            "fish".into(),
            ShellOverridesToml {
                source_lines: vec!["starship init fish | source".into()],
                ..Default::default()
            },
        );
        starship.shell.insert(
            "bash".into(),
            ShellOverridesToml {
                source_lines: vec!["eval \"$(starship init bash)\"".into()],
                ..Default::default()
            },
        );

        let raw = RawConfig {
            blocks: IndexMap::from([("starship".into(), starship)]),
        };

        let fish = resolve(&raw, Some("fish")).unwrap();
        assert_eq!(fish.blocks[0].actions.len(), 1);
        assert_eq!(
            fish.blocks[0].actions[0],
            Action::SourceLines {
                lines: vec!["starship init fish | source".into()],
            }
        );

        let bash = resolve(&raw, Some("bash")).unwrap();
        assert_eq!(bash.blocks[0].actions.len(), 1);
        assert_eq!(
            bash.blocks[0].actions[0],
            Action::SourceLines {
                lines: vec!["eval \"$(starship init bash)\"".into()],
            }
        );

        let resolution = resolve_with_details(&raw, Some("fish")).unwrap();
        let report = resolution
            .block_reports
            .iter()
            .find(|r| r.block_id == "starship")
            .unwrap();
        assert_eq!(report.source_line_count, 1);
        assert_eq!(report.action_count, 1);
    }
}
