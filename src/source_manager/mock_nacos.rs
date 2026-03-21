use crate::error::ConfigError;
use crate::source_manager::RawProvider;
use std::sync::{Arc, RwLock};
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct MockNacosRawProvider {
    pub node_id: String,
    pub cache: Arc<RwLock<String>>,
    pub simulated_updates: Vec<(u64, String)>,
}

impl MockNacosRawProvider {
    pub fn new(node_id: String, initial_data: String, simulated_updates: Vec<(u64, String)>) -> Self {
        Self {
            node_id,
            cache: Arc::new(RwLock::new(initial_data)),
            simulated_updates,
        }
    }
}

#[async_trait]
impl RawProvider for MockNacosRawProvider {
    fn node_id(&self) -> String {
        self.node_id.clone()
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        let cache = self.cache.read().unwrap();
        let mut hasher = DefaultHasher::new();
        cache.hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn fetch(&self) -> Result<String, ConfigError> {
        let raw_text = {
            let cache = self.cache.read().unwrap();
            cache.clone()
        };
        Ok(raw_text)
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
