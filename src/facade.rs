use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey, RawItemKey};
use crate::providers::CfgProviders;
use arc_swap::ArcSwap;
use serde::de::DeserializeOwned;
use std::any::TypeId;
use std::sync::Arc;

pub trait ConfigBind: DeserializeOwned + Send + Sync + 'static {
    // PATH is like the prefix in Spring Boot
    const PATH: Option<&'static str> = None;
}

pub struct ConfigEngineBuilder {
    providers: CfgProviders,
    global_sources: Vec<RawItemKey>,
    raw_store: std::collections::HashMap<RawItemKey, String>,
}

impl ConfigEngineBuilder {
    pub fn new() -> Self {
        Self {
            providers: CfgProviders::default(),
            global_sources: Vec::new(),
            raw_store: std::collections::HashMap::new(),
        }
    }

    pub fn with_global_sources(mut self, sources: Vec<RawItemKey>) -> Self {
        self.global_sources = sources;
        self
    }

    pub fn with_raw_content(mut self, key: RawItemKey, content: &str) -> Self {
        self.raw_store.insert(key, content.to_string());
        self
    }

    pub fn build(self) -> ConfigEngine {
        let tcx = Arc::new(CfgCtxt::new(self.providers));
        
        {
            let mut registry = tcx.global_sources.write().unwrap();
            *registry = self.global_sources;
        }
        
        {
            let mut store = tcx.raw_store.write().unwrap();
            *store = self.raw_store;
        }

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
        };

        // Initial load
        let initial_val: Arc<T> = self.compute_typed::<T>(&key)?;
        let arc_swap = Arc::new(ArcSwap::new(initial_val));

        // Register updater for future reloads
        let weak_swap = Arc::downgrade(&arc_swap);
        let key_clone = key.clone();
        
        let updater = Box::new(move |tcx: &CfgCtxt| -> Result<(), ConfigError> {
            if let Some(swap) = weak_swap.upgrade() {
                // To do this via tcx query:
                // let new_val = tcx.typed_config::<T>(key_clone.clone())?;
                // But since we need to know the type T to deserialize, we can just call compute_typed directly.
                // Or better, use the tcx to get the subtree and deserialize here.
                
                let value_map = tcx.subtree(key_clone.subtree.clone())?;
                let typed: T = serde_json::from_value(value_map.as_ref().clone())
                    .map_err(ConfigError::Json)?;
                
                swap.store(Arc::new(typed));
                Ok(())
            } else {
                // If the arc_swap is dropped, we can ignore
                Ok(())
            }
        });

        self.updaters.write().unwrap().push(updater);

        Ok(arc_swap)
    }

    // A helper to compute typed config (acts like the typed_config provider but with known T)
    fn compute_typed<T: ConfigBind>(&self, key: &TypedNodeKey) -> Result<Arc<T>, ConfigError> {
        let value_map = self.tcx.subtree(key.subtree.clone())?;
        let typed: T = serde_json::from_value(value_map.as_ref().clone())
            .map_err(ConfigError::Json)?;
        Ok(Arc::new(typed))
    }

    /// Triggered by a background worker when e.g. Nacos updates
    pub fn update_raw_content(&self, key: RawItemKey, new_content: String) {
        {
            let mut store = self.tcx.raw_store.write().unwrap();
            store.insert(key.clone(), new_content);
        }
        
        // 1. Mark the key as dirty.
        self.tcx.invalidate_raw_item(key);

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
