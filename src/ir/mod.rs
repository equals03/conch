//! Shell-neutral intermediate representation (IR).
//!
//! The core resolver emits ordered blocks. Each block keeps its predicates
//! and actions. Providers translate those guards and actions into shell-native
//! syntax; they must not reinterpret ordering or conflict rules.

use crate::config::EnvValue;
use crate::predicate::Predicate;

/// A single PATH mutation in evaluation order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathOp {
    Prepend(String),
    Append(String),
    MoveFront(String),
    MoveBack(String),
}

/// One shell-neutral action contributed by a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SetEnv {
        key: String,
        value: EnvValue,
    },
    SetAlias {
        name: String,
        value: String,
    },
    Path(PathOp),
    /// Literal lines to emit verbatim after structured actions (from shell overrides).
    SourceLines {
        lines: Vec<String>,
    },
}

/// One ordered contribution block (identified by `block_id` from config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub block_id: String,
    pub when: Vec<Predicate>,
    pub requires: Vec<Predicate>,
    pub actions: Vec<Action>,
}

/// Final ordered IR for a target shell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedIr {
    pub blocks: Vec<Block>,
}

impl ResolvedIr {
    pub fn push_block(&mut self, block: Block) {
        self.blocks.push(block);
    }
}
