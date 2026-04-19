//! Shell providers render shell-neutral IR into concrete shell syntax.
//!
//! Providers must not reimplement graph ordering or conflict semantics.

mod bash;
mod fish;

pub use bash::BashProvider;
pub use fish::FishProvider;

use crate::ir::{Block, ResolvedIr};
use crate::predicate::{Predicate, PredicateAtom};

pub trait ShellProvider {
    fn render(&self, ir: &ResolvedIr) -> String;
}

fn normalize_home(value: &str) -> String {
    if value == "~" {
        "$HOME".into()
    } else if let Some(rest) = value.strip_prefix("~/") {
        format!("$HOME/{}", rest)
    } else {
        value.to_string()
    }
}

pub(crate) struct HoistedBlock<'a> {
    pub block: &'a Block,
    pub residual_when: Vec<Predicate>,
    pub residual_requires: Vec<Predicate>,
}

pub(crate) struct HoistRun<'a> {
    pub hoistable: Vec<Predicate>,
    pub blocks: Vec<HoistedBlock<'a>>,
}

pub(crate) fn build_hoist_runs(blocks: &[Block]) -> Vec<HoistRun<'_>> {
    let mut runs = Vec::new();

    for block in blocks {
        let (hoistable_when, residual_when) = split_hoistable(&block.when);
        let (hoistable_requires, residual_requires) = split_hoistable(&block.requires);
        let mut hoistable = hoistable_when;
        hoistable.extend(hoistable_requires);
        let key = canonical_hoistable_key(&hoistable);
        let hoisted = HoistedBlock {
            block,
            residual_when,
            residual_requires,
        };

        if key.is_empty() {
            runs.push(HoistRun {
                hoistable,
                blocks: vec![hoisted],
            });
            continue;
        }

        let can_extend = runs.last().is_some_and(|run| {
            !run.hoistable.is_empty() && canonical_hoistable_key(&run.hoistable) == key
        });

        match (can_extend, runs.last_mut()) {
            (true, Some(last)) => last.blocks.push(hoisted),
            _ => runs.push(HoistRun {
                hoistable,
                blocks: vec![hoisted],
            }),
        }
    }

    runs
}

fn split_hoistable(predicates: &[Predicate]) -> (Vec<Predicate>, Vec<Predicate>) {
    let mut hoistable = Vec::new();
    let mut residual = Vec::new();

    for predicate in predicates {
        if is_hoistable(predicate) {
            hoistable.push(predicate.clone());
        } else {
            residual.push(predicate.clone());
        }
    }

    (hoistable, residual)
}

fn is_hoistable(predicate: &Predicate) -> bool {
    matches!(
        predicate.atom,
        PredicateAtom::Interactive | PredicateAtom::Login
    )
}

fn canonical_hoistable_key(predicates: &[Predicate]) -> Vec<(u8, bool)> {
    let mut key: Vec<(u8, bool)> = predicates
        .iter()
        .map(|predicate| match &predicate.atom {
            PredicateAtom::Interactive => (0, predicate.negated),
            PredicateAtom::Login => (1, predicate.negated),
            _ => unreachable!(
                "canonical_hoistable_key must only receive interactive/login predicates"
            ),
        })
        .collect();
    key.sort_unstable();
    key
}

/// Emit `text` with `prefix` at the start and after every `\n`; append `\n` if `text` has no trailing newline.
pub(crate) fn push_indented_verbatim(out: &mut String, text: &str, prefix: &str) {
    out.push_str(prefix);
    for ch in text.chars() {
        if ch == '\n' {
            out.push('\n');
            out.push_str(prefix);
        } else {
            out.push(ch);
        }
    }
    if !text.ends_with('\n') {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Action;

    fn predicate(atom: PredicateAtom) -> Predicate {
        Predicate {
            negated: false,
            atom,
        }
    }

    fn negated(atom: PredicateAtom) -> Predicate {
        Predicate {
            negated: true,
            atom,
        }
    }

    fn make_block(block_id: &str, when: Vec<Predicate>, requires: Vec<Predicate>) -> Block {
        Block {
            block_id: block_id.into(),
            when,
            requires,
            actions: vec![Action::SetAlias {
                name: block_id.into(),
                value: block_id.into(),
            }],
        }
    }

    #[test]
    fn groups_adjacent_blocks_with_same_canonical_hoistable_key() {
        let blocks = vec![
            make_block(
                "a",
                vec![
                    predicate(PredicateAtom::Interactive),
                    predicate(PredicateAtom::Login),
                ],
                vec![],
            ),
            make_block(
                "b",
                vec![predicate(PredicateAtom::Login)],
                vec![predicate(PredicateAtom::Interactive)],
            ),
        ];

        let runs = build_hoist_runs(&blocks);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].blocks.len(), 2);
        assert_eq!(runs[0].blocks[0].block.block_id, "a");
        assert_eq!(runs[0].blocks[1].block.block_id, "b");
        assert_eq!(
            canonical_hoistable_key(&runs[0].hoistable),
            vec![(0, false), (1, false)]
        );
    }

    #[test]
    fn preserves_residual_predicates_inside_hoisted_blocks() {
        let blocks = vec![make_block(
            "nvim",
            vec![predicate(PredicateAtom::Interactive)],
            vec![predicate(PredicateAtom::Command("nvim".into()))],
        )];

        let runs = build_hoist_runs(&blocks);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].hoistable.len(), 1);
        assert_eq!(runs[0].blocks[0].residual_when, Vec::<Predicate>::new());
        assert_eq!(
            runs[0].blocks[0].residual_requires,
            vec![predicate(PredicateAtom::Command("nvim".into()))]
        );
    }

    #[test]
    fn does_not_merge_different_hoistable_combinations() {
        let blocks = vec![
            make_block("a", vec![predicate(PredicateAtom::Interactive)], vec![]),
            make_block(
                "b",
                vec![
                    predicate(PredicateAtom::Interactive),
                    predicate(PredicateAtom::Login),
                ],
                vec![],
            ),
        ];

        let runs = build_hoist_runs(&blocks);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            canonical_hoistable_key(&runs[0].hoistable),
            vec![(0, false)]
        );
        assert_eq!(
            canonical_hoistable_key(&runs[1].hoistable),
            vec![(0, false), (1, false)]
        );
    }

    #[test]
    fn does_not_merge_non_adjacent_matching_runs() {
        let blocks = vec![
            make_block("a", vec![predicate(PredicateAtom::Interactive)], vec![]),
            make_block(
                "middle",
                vec![],
                vec![predicate(PredicateAtom::Command("git".into()))],
            ),
            make_block("b", vec![predicate(PredicateAtom::Interactive)], vec![]),
        ];

        let runs = build_hoist_runs(&blocks);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].blocks[0].block.block_id, "a");
        assert_eq!(runs[1].blocks[0].block.block_id, "middle");
        assert_eq!(runs[2].blocks[0].block.block_id, "b");
    }

    #[test]
    fn keeps_negated_hoistable_guards_in_a_shared_run() {
        let blocks = vec![
            make_block("a", vec![negated(PredicateAtom::Interactive)], vec![]),
            make_block("b", vec![negated(PredicateAtom::Interactive)], vec![]),
        ];

        let runs = build_hoist_runs(&blocks);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].blocks.len(), 2);
        assert_eq!(canonical_hoistable_key(&runs[0].hoistable), vec![(0, true)]);
    }
}
