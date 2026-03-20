use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("JSON Parse Error: {0}")]
    Json(#[from] serde_json::Error),
    
    #[error("YAML Parse Error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    
    #[error("TOML Parse Error: {0}")]
    Toml(#[from] toml::de::Error),
    
    #[error("Provider Error: {0}")]
    Provider(String),
    
    #[error("Key not found in cache or provider failed: {0}")]
    NotFound(String),
    
    #[error("Unknown error: {0}")]
    Unknown(String),
}
