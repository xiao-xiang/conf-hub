use crate::error::ConfigError;
use crate::source_manager::ConfigNodeProvider;
use crate::parsers;
use serde_json::Value as ValueMap;
use std::sync::Arc;
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct EnvProvider {
    pub prefix: String,
}

impl EnvProvider {
    pub fn new(prefix: String) -> Self {
        Self { prefix }
    }
}

#[async_trait]
impl ConfigNodeProvider for EnvProvider {
    fn node_id(&self) -> String {
        "env://global".to_string()
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        let mut hasher = DefaultHasher::new();
        for (key, val) in std::env::vars() {
            if key.starts_with(&self.prefix) {
                key.hash(&mut hasher);
                val.hash(&mut hasher);
            }
        }
        Ok(hasher.finish())
    }

    fn fetch_and_parse(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let mut map = std::collections::HashMap::new();
        for (key, val) in std::env::vars() {
            if key.starts_with(&self.prefix) {
                let k = key[self.prefix.len()..].to_string();
                map.insert(k, val);
            }
        }
        let json_str = serde_json::to_string(&map).unwrap();
        Ok(Arc::new(parsers::parse_env(&json_str)))
    }

    async fn watch(&self, _on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        Ok(())
    }
}
