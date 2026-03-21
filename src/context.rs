use crate::dep_graph::{DepGraph, DepNode, NodeState};
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey};
use crate::providers::CfgProviders;
use crate::source_manager::ConfigNodeProvider;
use serde_json::Value as ValueMap;
use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};

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

thread_local! {
    static ACTIVE_QUERY_STACK: RefCell<Vec<DepNode>> = const { RefCell::new(Vec::new()) };
}

struct QueryStackGuard;

impl Drop for QueryStackGuard {
    fn drop(&mut self) {
        ACTIVE_QUERY_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

pub struct CachedResult {
    pub value: Arc<dyn Any + Send + Sync>,
    pub fingerprint: Fingerprint,
}

pub struct CfgCtxt {
    cache: RwLock<HashMap<DepNode, CachedResult>>,
    dep_graph: RwLock<DepGraph>,
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
            cache: RwLock::new(HashMap::new()),
            dep_graph: RwLock::new(DepGraph::new()),
            providers,
            node_providers,
            global_source_ids,
        }
    }

    fn track_dependency(&self, target: &DepNode) {
        ACTIVE_QUERY_STACK.with(|stack| {
            if let Some(caller) = stack.borrow().last() {
                let mut graph = self.dep_graph.write().unwrap();
                graph.add_edge(target.clone(), caller.clone());
            }
        });
    }

    fn push_active_query(node: DepNode) -> QueryStackGuard {
        ACTIVE_QUERY_STACK.with(|stack| {
            stack.borrow_mut().push(node);
        });
        QueryStackGuard
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

        self.dep_graph.write().unwrap().clear_edges(&node);
        let _guard = Self::push_active_query(node.clone());

        let result = f();

        match result {
            Ok((val, fingerprint)) => {
                self.cache.write().unwrap().insert(
                    node.clone(),
                    CachedResult {
                        value: val.clone() as Arc<dyn Any + Send + Sync>,
                        fingerprint,
                    },
                );

                self.dep_graph.write().unwrap().set_state(node, NodeState::Green);

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
        self.dep_graph.write().unwrap().mark_dirty(&node);
    }

    pub fn is_typed_node_dirty(&self, key: &TypedNodeKey) -> bool {
        let node = DepNode::Typed(key.clone());
        self.dep_graph.read().unwrap().get_state(&node) != NodeState::Green
    }
}
