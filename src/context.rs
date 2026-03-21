use crate::dep_graph::{DepGraph, DepNode, NodeState};
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey};
use crate::providers::CfgProviders;
use crate::source_manager::ConfigNodeProvider;
use serde_json::Value as ValueMap;
use std::any::Any;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use dashmap::DashMap;

type Fingerprint = u64;

fn calculate_value_fingerprint(value: &ValueMap) -> Fingerprint {
    fn hash_value(value: &ValueMap, hasher: &mut DefaultHasher) {
        match value {
            ValueMap::Null => 0u8.hash(hasher),
            ValueMap::Bool(v) => {
                1u8.hash(hasher);
                v.hash(hasher);
            }
            ValueMap::Number(v) => {
                2u8.hash(hasher);
                v.to_string().hash(hasher);
            }
            ValueMap::String(v) => {
                3u8.hash(hasher);
                v.hash(hasher);
            }
            ValueMap::Array(items) => {
                4u8.hash(hasher);
                items.len().hash(hasher);
                for item in items {
                    hash_value(item, hasher);
                }
            }
            ValueMap::Object(map) => {
                5u8.hash(hasher);
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_unstable();
                for key in keys {
                    key.hash(hasher);
                    if let Some(v) = map.get(key) {
                        hash_value(v, hasher);
                    }
                }
            }
        }
    }

    let mut hasher = DefaultHasher::new();
    hash_value(value, &mut hasher);
    hasher.finish()
}

pub struct CachedResult {
    pub value: Arc<dyn Any + Send + Sync>,
    pub fingerprint: Fingerprint,
}

pub struct CfgCtxt {
    cache: DashMap<DepNode, CachedResult>,
    dep_graph: DepGraph,
    active_query_stack: RwLock<Vec<DepNode>>,
    pub providers: CfgProviders,
    pub node_providers: HashMap<String, Arc<dyn ConfigNodeProvider>>,
    pub global_source_ids: Vec<String>,
}

impl CfgCtxt {
    pub fn new(
        providers: CfgProviders,
        node_providers: HashMap<String, Arc<dyn ConfigNodeProvider>>,
        global_source_ids: Vec<String>,
    ) -> Self {
        Self {
            cache: DashMap::new(),
            dep_graph: DepGraph::new(),
            active_query_stack: RwLock::new(Vec::new()),
            providers,
            node_providers,
            global_source_ids,
        }
    }

    fn track_dependency(&self, target: &DepNode) {
        let stack = self.active_query_stack.read().unwrap();
        if let Some(caller) = stack.last() {
            self.dep_graph.add_edge(target.clone(), caller.clone());
        }
    }

    fn push_active_query(&self, node: DepNode) {
        let mut stack = self.active_query_stack.write().unwrap();
        stack.push(node);
    }

    fn pop_active_query(&self) {
        let mut stack = self.active_query_stack.write().unwrap();
        stack.pop();
    }

    fn with_query<T, F>(&self, node: DepNode, f: F) -> Result<(Arc<T>, Fingerprint), ConfigError>
    where
        T: Send + Sync + 'static,
        F: FnOnce() -> Result<(Arc<T>, Fingerprint), ConfigError>,
    {
        self.track_dependency(&node);

        // Check cache and state
        let state = self.dep_graph.get_state(&node);
        if state == NodeState::Green {
            if let Some(cached) = self.cache.get(&node) {
                if let Ok(value) = cached.value.clone().downcast::<T>() {
                    return Ok((value, cached.fingerprint));
                }
            }
        }

        self.dep_graph.clear_edges(&node);
        self.push_active_query(node.clone());

        let result = f();
        
        self.pop_active_query();

        match result {
            Ok((val, fingerprint)) => {
                self.cache.insert(
                    node.clone(),
                    CachedResult {
                        value: val.clone() as Arc<dyn Any + Send + Sync>,
                        fingerprint,
                    },
                );

                self.dep_graph.set_state(node, NodeState::Green);

                Ok((val, fingerprint))
            }
            Err(e) => Err(e),
        }
    }

    pub fn source_ast(&self, node_id: String) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::SourceAST(node_id.clone());
        self.with_query(node, || {
            let provider = self.node_providers.get(&node_id)
                .ok_or_else(|| ConfigError::Provider(format!("Provider not found: {}", node_id)))?;
            let val = (self.providers.source_ast)(self, node_id)?;
            let fp = provider.raw_fingerprint()?;
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn merged_global(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::MergedGlobal;
        self.with_query(node, || {
            let val = (self.providers.merged_global)(self)?;
            let fp = calculate_value_fingerprint(&val);
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn resolved_global(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::ResolvedGlobal;
        self.with_query(node, || {
            let val = (self.providers.resolved_global)(self)?;
            let fp = calculate_value_fingerprint(&val);
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn subtree(&self, key: SubtreeKey) -> Result<Arc<ValueMap>, ConfigError> {
        let node = DepNode::Subtree(key.clone());
        self.with_query(node, || {
            let val = (self.providers.subtree)(self, key)?;
            let fp = calculate_value_fingerprint(&val);
            Ok((val, fp))
        }).map(|(v, _)| v)
    }

    pub fn typed_config<T: Send + Sync + 'static>(&self, key: TypedNodeKey) -> Result<Arc<T>, ConfigError> {
        let node = DepNode::Typed(key.clone());
        self.with_query(node, || {
            let val = (self.providers.typed_config)(self, key)?;
            let typed = val.downcast::<T>().map_err(|_| {
                ConfigError::Provider("typed config downcast failed".to_string())
            })?;
            Ok((typed, 0))
        }).map(|(v, _)| v)
    }

    pub fn invalidate_source(&self, node_id: String) {
        let node = DepNode::SourceAST(node_id);
        self.dep_graph.mark_dirty(&node);
    }

    pub fn is_typed_node_dirty(&self, key: &TypedNodeKey) -> bool {
        let node = DepNode::Typed(key.clone());
        self.dep_graph.get_state(&node) != NodeState::Green
    }
}
