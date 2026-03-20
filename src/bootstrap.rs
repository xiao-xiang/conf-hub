use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BootstrapConfig {
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    #[serde(rename = "file")]
    File {
        configs: Vec<FileConfigItem>,
    },
    #[serde(rename = "nacos")]
    Nacos {
        server_addr: String,
        namespace: Option<String>,
        username: Option<String>,
        password: Option<String>,
        configs: Vec<NacosConfigItem>,
    },
    #[serde(rename = "env")]
    Env {
        prefix: String,
    },
    #[serde(rename = "args")]
    Args,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileConfigItem {
    pub path: String,
    pub format: Option<String>, // "yaml", "toml", "json", "ini", "properties"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NacosConfigItem {
    pub data_id: String,
    pub group: String,
    #[serde(default)]
    pub dynamic: bool,
    pub file_extension: String,
}

impl BootstrapConfig {
    pub fn load_from_file(path: &str) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path).map_err(ConfigError::Io)?;
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("yaml");

        let config: BootstrapConfig = match ext {
            "yaml" | "yml" => serde_yaml::from_str(&content).map_err(ConfigError::Yaml)?,
            "json" => serde_json::from_str(&content).map_err(ConfigError::Json)?,
            "toml" => toml::from_str(&content).map_err(ConfigError::Toml)?,
            _ => {
                return Err(ConfigError::Provider(format!(
                    "Unsupported bootstrap format: {}",
                    ext
                )))
            }
        };

        Ok(config)
    }
}
