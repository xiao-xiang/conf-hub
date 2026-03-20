use crate::dep_graph::{DepGraph, DepNode, NodeState};
use crate::error::ConfigError;
use crate::keys::{RawItemKey, SubtreeKey, TypedNodeKey};
use crate::providers::CfgProviders;
use serde_json::Value as ValueMap;
use std::any::Any;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};

type Fingerprint = u64;

fn calculate_fingerprint<T: Hash>(value: &T) -> Fingerprint {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub struct CachedResult {
    pub value: Arc<dyn Any + Send + Sync>,
    pub fingerprint: Fingerprint,
}

pub struct CfgCtxt {
    cache: RwLock<HashMap<DepNode, CachedResult>>,
    dep_graph: RwLock<DepGraph>,
    pub providers: CfgProviders,
    // Thread-local stack to track the currently executing query for automatic dependency tracking
    active_query_stack: RwLock<Vec<DepNode>>,
    // The flat list of global sources defining the merge pipeline
    pub global_sources: RwLock<Vec<RawItemKey>>,
    // Store for dynamic/pushed raw contents (like Nacos)
    pub raw_store: RwLock<HashMap<RawItemKey, String>>,
}

impl Default for CfgCtxt {
    fn default() -> Self {
        Self::new(CfgProviders::default())
    }
}

impl CfgCtxt {
    pub fn new(providers: CfgProviders) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            dep_graph: RwLock::new(DepGraph::new()),
            providers,
            active_query_stack: RwLock::new(Vec::new()),
            global_sources: RwLock::new(Vec::new()),
            raw_store: RwLock::new(HashMap::new()),
        }
    }

    fn track_dependency(&self, target: &DepNode) {
        let stack = self.active_query_stack.read().unwrap();
        if let Some(caller) = stack.last() {
            let mut graph = self.dep_graph.write().unwrap();
            graph.add_edge(target.clone(), caller.clone());
        }
    }

    fn with_query<T, F>(&self, node: DepNode, f: F) -> Result<(Arc<T>, Fingerprint), ConfigError>
    where
        T: Send + Sync + 'static,
        F: FnOnce() -> Result<(Arc<T>, Fingerprint), ConfigError>,
    {
        self.track_dependency(&node);

        // Check cache and state
        let state = self.dep_graph.read().unwrap().get_state(&node);
        if state == NodeState::Green {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.get(&node) {
                if let Ok(value) = cached.value.clone().downcast::<T>() {
                    return Ok((value, cached.fingerprint));
                }
            }
        }

        // If unknown, we could do a more sophisticated check (early cutoff logic by checking dependencies recursively).
        // For simplicity in this skeleton, if it's not Green, we re-evaluate.
        // Before re-evaluating, clear old dependencies
        self.dep_graph.write().unwrap().clear_edges(&node);

        // Push to active stack
        self.active_query_stack.write().unwrap().push(node.clone());

        let result = f();

        // Pop from active stack
        self.active_query_stack.write().unwrap().pop();

        match result {
            Ok((val, fingerprint)) => {
                // Check early cutoff (if fingerprint is same, it's green and we don't need to propagate dirtiness further maybe? 
                // Actually the engine handles early cutoff here:
                let _is_changed = {
                    let cache = self.cache.read().unwrap();
                    if let Some(old) = cache.get(&node) {
                        old.fingerprint != fingerprint
                    } else {
                        true
                    }
                };

                // Update cache
                self.cache.write().unwrap().insert(
                    node.clone(),
                    CachedResult {
                        value: val.clone() as Arc<dyn Any + Send + Sync>,
                        fingerprint,
                    },
                );

                // Mark as green
                self.dep_graph.write().unwrap().set_state(node, NodeState::Green);

                Ok((val, fingerprint))
            }
            Err(e) => Err(e),
        }
    }

    // --- Query Facades ---

    pub fn raw_item(&self, key: RawItemKey) -> Result<Arc<String>, ConfigError> {
        let node = DepNode::RawItem(key.clone());
        self.with_query(node, || {
            let val = (self.providers.raw_item)(self, key)?;
            let fp = calculate_fingerprint(&*val);
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn parsed_item(&self, key: RawItemKey) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::ParsedItem(key.clone());
        self.with_query(node, || {
            let val = (self.providers.parsed_item)(self, key)?;
            let fp = calculate_fingerprint(&val.to_string()); // Simple hash of json string for ValueMap
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn merged_global(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::MergedGlobal;
        self.with_query(node, || {
            let val = (self.providers.merged_global)(self)?;
            let fp = calculate_fingerprint(&val.to_string());
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn resolved_global(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::ResolvedGlobal;
        self.with_query(node, || {
            let val = (self.providers.resolved_global)(self)?;
            let fp = calculate_fingerprint(&val.to_string());
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn subtree(&self, key: SubtreeKey) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::Subtree(key.clone());
        self.with_query(node, || {
            let val = (self.providers.subtree)(self, key)?;
            let fp = calculate_fingerprint(&val.to_string());
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn typed_config<T: Send + Sync + 'static>(&self, key: TypedNodeKey) -> Result<Arc<T>, ConfigError> {
        let node = DepNode::Typed(key.clone());
        self.with_query(node, || {
            let val = (self.providers.typed_config)(self, key)?;
            // T might not be Hash. But since it's the final output, we don't necessarily need its fingerprint
            // for downstream. We just use a dummy fingerprint 0.
            Ok((val.downcast::<T>().unwrap(), 0))
        }).map(|(v, _)| v)
    }

    // External event trigger
    pub fn invalidate_raw_item(&self, key: RawItemKey) {
        let node = DepNode::RawItem(key);
        self.dep_graph.write().unwrap().mark_dirty(&node);
    }
}
