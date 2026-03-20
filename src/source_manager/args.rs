use crate::error::ConfigError;
use crate::keys::RawItemKey;
use crate::source_manager::SourceConnector;

pub struct ArgsConnector {
    pub key: RawItemKey,
    pub mocked_args: Option<String>,
}

use async_trait::async_trait;

#[async_trait]
impl SourceConnector for ArgsConnector {
    async fn fetch_initial(&self) -> Result<std::collections::HashMap<RawItemKey, Option<String>>, ConfigError> {
        let mut map = std::collections::HashMap::new();
        if let Some(ref args) = self.mocked_args {
            map.insert(self.key.clone(), Some(args.clone()));
        } else {
            map.insert(self.key.clone(), Some("[]".to_string()));
        }
        Ok(map)
    }

    async fn watch(&self, _on_update: Box<dyn Fn(RawItemKey, String) + Send + Sync>) -> Result<(), ConfigError> {
        Ok(())
    }

    fn keys(&self) -> Vec<RawItemKey> {
        vec![self.key.clone()]
    }
}
