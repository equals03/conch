//! DAG construction for declared blocks.
//!
//! Ordering is computed across declared blocks. References to unknown block ids are
//! validation errors because they are almost always typos.

use std::collections::{BTreeSet, HashMap, HashSet};

use petgraph::algo::{has_path_connecting, toposort};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;

use crate::config::{BlockConfig, Config};
use crate::error::ConchError;

#[derive(Debug)]
pub struct BlockGraph {
    graph: DiGraph<String, ()>,
    positions: HashMap<NodeIndex, usize>,
    indices: HashMap<String, NodeIndex>,
}

impl BlockGraph {
    pub fn topo_order(&self) -> Result<Vec<String>, ConchError> {
        let mut indegree_zero = BTreeSet::new();
        let mut indegree = HashMap::new();

        for node in self.graph.node_indices() {
            let count = self
                .graph
                .neighbors_directed(node, Direction::Incoming)
                .count();
            indegree.insert(node, count);
            if count == 0 {
                indegree_zero.insert(self.sort_key(node));
            }
        }

        let mut order = Vec::new();
        while let Some(key) = indegree_zero.pop_first() {
            let node = NodeIndex::new(key.2);
            order.push(self.graph[node].clone());

            let outgoing: Vec<_> = self
                .graph
                .neighbors_directed(node, Direction::Outgoing)
                .collect();
            for neighbor in outgoing {
                let Some(count) = indegree.get_mut(&neighbor) else {
                    return Err(ConchError::Graph(
                        "internal error: topological sort state is inconsistent".into(),
                    ));
                };
                *count -= 1;
                if *count == 0 {
                    indegree_zero.insert(self.sort_key(neighbor));
                }
            }
        }

        if order.len() != self.graph.node_count() {
            let cycle = toposort(&self.graph, None)
                .err()
                .and_then(|err| self.cycle_path(err.node_id()))
                .unwrap_or_else(|| vec!["unknown".into()]);
            return Err(ConchError::Graph(format!(
                "cycle detected in block ordering: {}",
                cycle.join(" -> ")
            )));
        }

        Ok(order)
    }

    pub fn ordered_before(&self, earlier: &str, later: &str) -> bool {
        let Some(&left) = self.indices.get(earlier) else {
            return false;
        };
        let Some(&right) = self.indices.get(later) else {
            return false;
        };
        has_path_connecting(&self.graph, left, right, None)
    }

    fn sort_key(&self, node: NodeIndex) -> (usize, String, usize) {
        (
            *self.positions.get(&node).unwrap_or(&usize::MAX),
            self.graph[node].clone(),
            node.index(),
        )
    }

    fn cycle_path(&self, start: NodeIndex) -> Option<Vec<String>> {
        let mut path = vec![start];
        let mut seen = HashSet::from([start]);
        if self.find_cycle_from(start, start, &mut path, &mut seen) {
            return Some(
                path.into_iter()
                    .map(|node| self.graph[node].clone())
                    .collect(),
            );
        }
        None
    }

    fn find_cycle_from(
        &self,
        current: NodeIndex,
        target: NodeIndex,
        path: &mut Vec<NodeIndex>,
        seen: &mut HashSet<NodeIndex>,
    ) -> bool {
        for neighbour in self.graph.neighbors_directed(current, Direction::Outgoing) {
            if neighbour == target {
                path.push(neighbour);
                return true;
            }

            if seen.insert(neighbour) {
                path.push(neighbour);
                if self.find_cycle_from(neighbour, target, path, seen) {
                    return true;
                }
                path.pop();
                seen.remove(&neighbour);
            }
        }
        false
    }
}

pub fn build_graph(config: &Config, block_ids: &[String]) -> Result<BlockGraph, ConchError> {
    let block_id_set: BTreeSet<&str> = block_ids.iter().map(|id| id.as_str()).collect();

    for (block_id, block) in &config.blocks {
        validate_references(config, block_id, block)?;
    }

    let mut graph = DiGraph::<String, ()>::new();
    let mut indices = HashMap::new();
    let mut positions = HashMap::new();

    for (position, block_id) in block_ids.iter().enumerate() {
        if indices.contains_key(block_id) {
            return Err(ConchError::Graph(format!(
                "duplicate block id `{block_id}` in resolution order"
            )));
        }
        if !config.blocks.contains_key(block_id) {
            return Err(ConchError::Validation(format!(
                "resolution order references unknown block `{block_id}`"
            )));
        }
        let node = graph.add_node(block_id.clone());
        indices.insert(block_id.clone(), node);
        positions.insert(node, position);
    }

    for block_id in block_ids {
        let block = &config.blocks[block_id];
        let source = indices[block_id];

        for target_id in &block.before {
            if block_id_set.contains(target_id.as_str()) {
                let target = indices[target_id];
                if graph.find_edge(source, target).is_none() {
                    graph.add_edge(source, target, ());
                }
            }
        }

        for target_id in &block.after {
            if block_id_set.contains(target_id.as_str()) {
                let target = indices[target_id];
                if graph.find_edge(target, source).is_none() {
                    graph.add_edge(target, source, ());
                }
            }
        }
    }

    Ok(BlockGraph {
        graph,
        positions,
        indices,
    })
}

fn validate_references(
    config: &Config,
    block_id: &str,
    block: &BlockConfig,
) -> Result<(), ConchError> {
    for target_id in &block.before {
        validate_reference_target(config, block_id, target_id, "before")?;
    }
    for target_id in &block.after {
        validate_reference_target(config, block_id, target_id, "after")?;
    }
    Ok(())
}

fn validate_reference_target(
    config: &Config,
    block_id: &str,
    target_id: &str,
    field: &str,
) -> Result<(), ConchError> {
    if target_id == block_id {
        return Err(ConchError::Graph(format!(
            "block `{block_id}` cannot reference itself in `{field}`"
        )));
    }
    if !config.blocks.contains_key(target_id) {
        return Err(ConchError::Validation(format!(
            "block `{block_id}` references unknown block `{target_id}` in `{field}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::config::{BlockConfig, Config, PathSpec, ShellOverride};

    fn block() -> BlockConfig {
        BlockConfig {
            when: Vec::new(),
            requires: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            env: IndexMap::new(),
            alias: IndexMap::new(),
            path: PathSpec::default(),
            shell: IndexMap::<String, ShellOverride>::new(),
        }
    }

    #[test]
    fn builds_topological_order() {
        let mut base = block();
        base.before.push("nvim".into());

        let mut nvim = block();
        nvim.after.push("base".into());

        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([
                ("base".into(), base),
                ("nvim".into(), nvim),
                ("git".into(), block()),
            ]),
        };

        let graph = build_graph(&config, &["base".into(), "nvim".into(), "git".into()]).unwrap();
        let order = graph.topo_order().unwrap();

        assert_eq!(order, vec!["base", "nvim", "git"]);
        assert!(graph.ordered_before("base", "nvim"));
        assert!(!graph.ordered_before("git", "base"));
    }

    #[test]
    fn ignores_edges_to_absent_blocks() {
        let mut base = block();
        base.before.push("nvim".into());

        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([("base".into(), base), ("nvim".into(), block())]),
        };

        let graph = build_graph(&config, &["base".into()]).unwrap();
        let order = graph.topo_order().unwrap();
        assert_eq!(order, vec!["base"]);
    }

    #[test]
    fn reports_cycles_with_path() {
        let mut a = block();
        a.before.push("b".into());
        let mut b = block();
        b.before.push("c".into());
        let mut c = block();
        c.before.push("a".into());

        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([("a".into(), a), ("b".into(), b), ("c".into(), c)]),
        };

        let graph = build_graph(&config, &["a".into(), "b".into(), "c".into()]).unwrap();
        let err = graph.topo_order().unwrap_err().to_string();
        assert!(err.contains("cycle detected in block ordering"));
        assert!(err.contains("a") || err.contains("b") || err.contains("c"));
    }

    #[test]
    fn rejects_unknown_references() {
        let mut a = block();
        a.before.push("missing".into());

        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([("a".into(), a)]),
        };

        let err = build_graph(&config, &["a".into()]).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("in `before`"));
    }

    #[test]
    fn rejects_self_references() {
        let mut a = block();
        a.after.push("a".into());

        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([("a".into(), a)]),
        };

        let err = build_graph(&config, &["a".into()]).unwrap_err();
        assert!(matches!(err, ConchError::Graph(_)));
        assert!(err.to_string().contains("cannot reference itself"));
    }

    #[test]
    fn rejects_duplicate_block_ids_in_order() {
        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([("a".into(), block()), ("b".into(), block())]),
        };

        let err = build_graph(&config, &["a".into(), "b".into(), "a".into()]).unwrap_err();
        assert!(matches!(err, ConchError::Graph(_)));
        assert!(err.to_string().contains("duplicate block id"));
    }

    #[test]
    fn rejects_unknown_block_ids_in_order() {
        let config = Config {
            init: Default::default(),
            blocks: IndexMap::from([("a".into(), block())]),
        };

        let err = build_graph(&config, &["a".into(), "missing".into()]).unwrap_err();
        assert!(matches!(err, ConchError::Validation(_)));
        assert!(err.to_string().contains("unknown block `missing`"));
    }
}
