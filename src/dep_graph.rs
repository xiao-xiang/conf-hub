use crate::keys::{RawItemKey, SubtreeKey, TypedNodeKey};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum DepNode {
    RawItem(RawItemKey),
    ParsedItem(RawItemKey),
    MergedGlobal,
    ResolvedGlobal,
    Subtree(SubtreeKey),
    Typed(TypedNodeKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Green,   // Up to date
    Red,     // Known to be changed (dirty)
    Unknown, // Needs to be checked
}

#[derive(Debug)]
pub struct DepGraph {
    /// Maps a node to its current state
    pub states: HashMap<DepNode, NodeState>,
    /// Forward edges: node -> nodes that depend on it (for invalidation)
    pub forward_edges: HashMap<DepNode, HashSet<DepNode>>,
    /// Backward edges: node -> nodes it depends on (for recalculation/checking)
    pub backward_edges: HashMap<DepNode, HashSet<DepNode>>,
}

impl DepGraph {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            forward_edges: HashMap::new(),
            backward_edges: HashMap::new(),
        }
    }

    pub fn add_edge(&mut self, from: DepNode, to: DepNode) {
        self.forward_edges
            .entry(from.clone())
            .or_default()
            .insert(to.clone());
        self.backward_edges
            .entry(to)
            .or_default()
            .insert(from);
    }

    pub fn clear_edges(&mut self, node: &DepNode) {
        if let Some(deps) = self.backward_edges.remove(node) {
            for dep in deps {
                if let Some(forward) = self.forward_edges.get_mut(&dep) {
                    forward.remove(node);
                }
            }
        }
    }

    pub fn mark_dirty(&mut self, node: &DepNode) {
        self.states.insert(node.clone(), NodeState::Red);
        let mut queue = vec![node.clone()];
        let mut visited = HashSet::new();
        visited.insert(node.clone());

        while let Some(current) = queue.pop() {
            if let Some(dependents) = self.forward_edges.get(&current) {
                for dep in dependents {
                    if visited.insert(dep.clone()) {
                        // Mark dependents as Unknown (they need to re-evaluate if their inputs actually changed)
                        self.states.insert(dep.clone(), NodeState::Unknown);
                        queue.push(dep.clone());
                    }
                }
            }
        }
    }

    pub fn get_state(&self, node: &DepNode) -> NodeState {
        self.states.get(node).copied().unwrap_or(NodeState::Unknown)
    }

    pub fn set_state(&mut self, node: DepNode, state: NodeState) {
        self.states.insert(node, state);
    }
}
