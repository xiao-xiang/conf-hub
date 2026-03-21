use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey};
use crate::providers::CfgProviders;
use crate::source_manager::ConfigNodeProvider;
use arc_swap::ArcSwap;
use serde::de::DeserializeOwned;
use std::any::TypeId;
use std::sync::Arc;

pub trait ConfigBind: DeserializeOwned + Send + Sync + 'static {
    // PATH is like the prefix in Spring Boot
    const PATH: Option<&'static str> = None;

    // A factory method for type-erased deserialization
    fn deserialize_any(val: &serde_json::Value) -> Result<Arc<dyn std::any::Any + Send + Sync>, ConfigError> {
        let typed: Self = serde_json::from_value(val.clone()).map_err(ConfigError::Json)?;
        Ok(Arc::new(typed))
    }
}

pub struct ConfigEngineBuilder {
    providers: CfgProviders,
    node_providers: std::collections::HashMap<String, Arc<dyn ConfigNodeProvider>>,
    global_source_ids: Vec<String>,
}

impl ConfigEngineBuilder {
    pub fn new() -> Self {
        Self {
            providers: CfgProviders::default(),
            node_providers: std::collections::HashMap::new(),
            global_source_ids: Vec::new(),
        }
    }

    pub fn add_provider(mut self, provider: Arc<dyn ConfigNodeProvider>) -> Self {
        let id = provider.node_id();
        self.node_providers.insert(id.clone(), provider);
        self.global_source_ids.push(id);
        self
    }

    pub fn build(self) -> ConfigEngine {
        let tcx = Arc::new(CfgCtxt::new(self.providers, self.node_providers, self.global_source_ids));
        
        ConfigEngine {
            tcx,
            updaters: std::sync::RwLock::new(Vec::new()),
        }
    }
}

pub struct ConfigEngine {
    tcx: Arc<CfgCtxt>,
    updaters: std::sync::RwLock<Vec<Box<dyn Fn(&CfgCtxt) -> Result<(), ConfigError> + Send + Sync>>>,
}

impl ConfigEngine {
    pub fn builder() -> ConfigEngineBuilder {
        ConfigEngineBuilder::new()
    }

    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn tcx(&self) -> Arc<CfgCtxt> {
        self.tcx.clone()
    }

    pub fn load<T: ConfigBind>(&self) -> Result<Arc<ArcSwap<T>>, ConfigError> {
        let key = TypedNodeKey {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            subtree: SubtreeKey {
                path: T::PATH.map(|s| s.to_string()),
            },
            deserializer: <T as ConfigBind>::deserialize_any,
        };

        // Initial load using tcx query system to leverage cache and graph!
        let initial_val: Arc<T> = self.tcx.typed_config::<T>(key.clone())?;
        let arc_swap = Arc::new(ArcSwap::new(initial_val));

        // Register updater for future reloads
        let weak_swap = Arc::downgrade(&arc_swap);
        let key_clone = key.clone();
        
        let updater = Box::new(move |tcx: &CfgCtxt| -> Result<(), ConfigError> {
            if let Some(swap) = weak_swap.upgrade() {
                // Now using the proper query system to get typed config
                let new_val = tcx.typed_config::<T>(key_clone.clone())?;
                swap.store(new_val);
                Ok(())
            } else {
                // If the arc_swap is dropped, we can ignore
                Ok(())
            }
        });

        self.updaters.write().unwrap().push(updater);

        Ok(arc_swap)
    }

    /// Triggered by a background worker when e.g. Nacos updates
    pub fn update_source(&self, node_id: String) {
        // 1. Mark the key as dirty.
        self.tcx.invalidate_source(node_id);

        // 2. Trigger reload for all registered handlers
        self.reload_all();
    }

    pub fn reload_all(&self) {
        let updaters = self.updaters.read().unwrap();
        for updater in updaters.iter() {
            // If early cutoff happens, tcx.subtree will return the cached Arc<ValueMap> 
            // very quickly without doing the work.
            let _ = updater(&self.tcx);
        }
    }
}
