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

    // A factory method for type-erased deserialization
    fn deserialize_any(val: &serde_json::Value) -> Result<Arc<dyn std::any::Any + Send + Sync>, ConfigError> {
        let typed: Self = serde_json::from_value(val.clone()).map_err(ConfigError::Json)?;
        Ok(Arc::new(typed))
    }
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
