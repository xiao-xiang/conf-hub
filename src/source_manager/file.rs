use crate::error::ConfigError;
use crate::source_manager::ConfigNodeProvider;
use crate::parsers;
use serde_json::Value as ValueMap;
use std::fs;
use std::sync::Arc;
use async_trait::async_trait;
use std::time::SystemTime;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct FileProvider {
    pub path: String,
    pub format: String,
}

impl FileProvider {
    pub fn new(path: String, format: String) -> Self {
        Self { path, format }
    }
}

#[async_trait]
impl ConfigNodeProvider for FileProvider {
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

    fn fetch_and_parse(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let raw_text = fs::read_to_string(&self.path).map_err(ConfigError::Io)?;
        if raw_text.trim().is_empty() {
            return Ok(Arc::new(ValueMap::Object(serde_json::Map::new())));
        }

        let parsed_value = match self.format.as_str() {
            "yaml" | "yml" => serde_yaml::from_str(&raw_text).map_err(ConfigError::Yaml)?,
            "toml" => toml::from_str(&raw_text).map_err(ConfigError::Toml)?,
            "json" => serde_json::from_str(&raw_text).map_err(ConfigError::Json)?,
            "ini" => parsers::parse_ini(&raw_text)?,
            "properties" => parsers::parse_properties(&raw_text)?,
            _ => ValueMap::Null,
        };

        Ok(Arc::new(parsed_value))
    }

    async fn watch(&self, _on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        // For files, we could implement a file watcher here using `notify` crate, 
        // but for now we just return Ok
        Ok(())
    }
}
