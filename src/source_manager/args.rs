use crate::error::ConfigError;
use crate::source_manager::ConfigNodeProvider;
use crate::parsers;
use serde_json::Value as ValueMap;
use std::sync::Arc;
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct ArgsProvider {
    pub mocked_args: Option<Vec<String>>,
}

impl ArgsProvider {
    pub fn new(mocked_args: Option<Vec<String>>) -> Self {
        Self { mocked_args }
    }
}

#[async_trait]
impl ConfigNodeProvider for ArgsProvider {
    fn node_id(&self) -> String {
        "args://global".to_string()
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        let mut hasher = DefaultHasher::new();
        if let Some(ref args) = self.mocked_args {
            args.hash(&mut hasher);
        } else {
            for arg in std::env::args() {
                arg.hash(&mut hasher);
            }
        }
        Ok(hasher.finish())
    }

    fn fetch_and_parse(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let args: Vec<String> = if let Some(ref mocked) = self.mocked_args {
            mocked.clone()
        } else {
            std::env::args().collect()
        };
        
        let json_str = serde_json::to_string(&args).unwrap();
        Ok(Arc::new(parsers::parse_args(&json_str)))
    }

    async fn watch(&self, _on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        Ok(())
    }
}
