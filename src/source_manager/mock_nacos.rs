use crate::error::ConfigError;
use crate::source_manager::ConfigNodeProvider;
use serde_json::Value as ValueMap;
use std::sync::{Arc, RwLock};
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct MockNacosProvider {
    pub node_id: String,
    pub cache: Arc<RwLock<String>>,
    pub simulated_updates: Vec<(u64, String)>,
    pub format: String,
}

impl MockNacosProvider {
    pub fn new(node_id: String, initial_data: String, format: String, simulated_updates: Vec<(u64, String)>) -> Self {
        Self {
            node_id,
            cache: Arc::new(RwLock::new(initial_data)),
            simulated_updates,
            format,
        }
    }
}

#[async_trait]
impl ConfigNodeProvider for MockNacosProvider {
    fn node_id(&self) -> String {
        self.node_id.clone()
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        let cache = self.cache.read().unwrap();
        let mut hasher = DefaultHasher::new();
        cache.hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn fetch_and_parse(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let raw_text = {
            let cache = self.cache.read().unwrap();
            cache.clone()
        };
        let parsed_value = match self.format.as_str() {
            "json" => serde_json::from_str(&raw_text).map_err(ConfigError::Json)?,
            "yaml" | "yml" => serde_yaml::from_str(&raw_text).map_err(ConfigError::Yaml)?,
            "toml" => toml::from_str(&raw_text).map_err(ConfigError::Toml)?,
            _ => ValueMap::Null,
        };
        Ok(Arc::new(parsed_value))
    }

    async fn watch(&self, on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        let node_id = self.node_id.clone();
        let updates = self.simulated_updates.clone();
        let cache = self.cache.clone();
        
        if !updates.is_empty() {
            tokio::spawn(async move {
                for (delay_ms, content) in updates {
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    {
                        let mut w = cache.write().unwrap();
                        *w = content;
                    }
                    on_update(node_id.clone());
                }
            });
        }
        Ok(())
    }
}
