use crate::error::ConfigError;
use crate::source_manager::RawProvider;
use std::fs;
use std::sync::Arc;
use async_trait::async_trait;
use std::time::SystemTime;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct FileRawProvider {
    pub path: String,
}

impl FileRawProvider {
    pub fn new(path: String) -> Self {
        Self { path }
    }
}

#[async_trait]
impl RawProvider for FileRawProvider {
    fn node_id(&self) -> String {
        format!("file://{}", self.path)
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        let meta = fs::metadata(&self.path).map_err(ConfigError::Io)?;
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let mut hasher = DefaultHasher::new();
        mtime.hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn fetch(&self) -> Result<String, ConfigError> {
        fs::read_to_string(&self.path).map_err(ConfigError::Io)
    }

    async fn watch(&self, _on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        Ok(())
    }
}
