use crate::error::ConfigError;
use crate::keys::RawItemKey;
use crate::source_manager::SourceConnector;

pub struct EnvConnector {
    pub key: RawItemKey,
    // Provide mocked env for testing
    pub mocked_env: Option<String>,
}

use async_trait::async_trait;

#[async_trait]
impl SourceConnector for EnvConnector {
    async fn fetch_initial(&self) -> Result<std::collections::HashMap<RawItemKey, Option<String>>, ConfigError> {
        let prefix = self.key.uri.trim_start_matches("env://");
        let mut env_map = std::collections::HashMap::new();

        if let Some(ref env) = self.mocked_env {
            // parse mock and filter
            if let Ok(parsed) = serde_json::from_str::<std::collections::HashMap<String, String>>(env) {
                for (k, v) in parsed {
                    if k.starts_with(prefix) {
                        env_map.insert(k[prefix.len()..].to_string(), v);
                    }
                }
            }
        } else {
            // Real implementation
            for (k, v) in std::env::vars() {
                if k.starts_with(prefix) {
                    env_map.insert(k[prefix.len()..].to_string(), v);
                }
            }
        }
        
        let json = serde_json::to_string(&env_map).unwrap_or_else(|_| "{}".to_string());
        
        let mut results = std::collections::HashMap::new();
        results.insert(self.key.clone(), Some(json));
        Ok(results)
    }

    async fn watch(&self, _on_update: Box<dyn Fn(RawItemKey, String) + Send + Sync>) -> Result<(), ConfigError> {
        // Environment variables usually don't change at runtime, so no watch needed
        Ok(())
    }

    fn keys(&self) -> Vec<RawItemKey> {
        vec![self.key.clone()]
    }
}
