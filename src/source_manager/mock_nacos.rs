use crate::error::ConfigError;
use crate::keys::RawItemKey;
use crate::source_manager::SourceConnector;
use std::thread;
use std::time::Duration;

/// A mock implementation of NacosSourceManager for demonstration purposes.
pub struct MockNacosConnector {
    pub key: RawItemKey,
    pub server_addr: String,
    pub initial_data: String,
    pub simulated_updates: Vec<(u64, String)>, // (delay_ms, new_content)
}

use async_trait::async_trait;

#[async_trait]
impl SourceConnector for MockNacosConnector {
    async fn fetch_initial(&self) -> Result<std::collections::HashMap<RawItemKey, Option<String>>, ConfigError> {
        let mut results = std::collections::HashMap::new();
        results.insert(self.key.clone(), Some(self.initial_data.clone()));
        Ok(results)
    }

    async fn watch(&self, on_update: Box<dyn Fn(RawItemKey, String) + Send + Sync>) -> Result<(), ConfigError> {
        let key = self.key.clone();
        let updates = self.simulated_updates.clone();
        
        // Spawn a background thread to simulate Nacos pushing updates
        if !updates.is_empty() {
            thread::spawn(move || {
                for (delay_ms, content) in updates {
                    thread::sleep(Duration::from_millis(delay_ms));
                    on_update(key.clone(), content);
                }
            });
        }
        Ok(())
    }

    fn keys(&self) -> Vec<RawItemKey> {
        vec![self.key.clone()]
    }
}
