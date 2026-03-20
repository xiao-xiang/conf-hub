use crate::bootstrap::{BootstrapConfig, SourceConfig};
use crate::error::ConfigError;
use crate::facade::ConfigEngine;
use crate::keys::RawItemKey;
use crate::source_manager::args::ArgsConnector;
use crate::source_manager::env::EnvConnector;
use crate::source_manager::nacos::NacosConnector;
use crate::source_manager::SourceConnector;
use std::sync::Arc;

pub struct Bootstrapper {
    config: BootstrapConfig,
    connectors: Vec<Box<dyn SourceConnector>>,
}

impl Bootstrapper {
    pub fn new(config: BootstrapConfig) -> Self {
        Self {
            config,
            connectors: Vec::new(),
        }
    }

    /// Add a custom connector (useful for injecting mocks during tests)
    pub fn add_connector(&mut self, connector: Box<dyn SourceConnector>) {
        self.connectors.push(connector);
    }

    pub async fn bootstrap(mut self) -> Result<Arc<ConfigEngine>, ConfigError> {
        let mut builder = ConfigEngine::builder();
        let mut global_keys: Vec<RawItemKey> = Vec::new();

        // Phase 1 & 2: Parse and build topology, and automatically instantiate connectors
        for source_cfg in &self.config.sources {
            match source_cfg {
                SourceConfig::File { configs } => {
                    for file_cfg in configs {
                        let fmt = match file_cfg.format.as_deref().unwrap_or("yaml") {
                            "yaml" | "yml" => "yaml",
                            "toml" => "toml",
                            "json" => "json",
                            "ini" => "ini",
                            "properties" => "properties",
                            _ => "yaml",
                        };
                        let key = RawItemKey::new(format!("file://{}", file_cfg.path), fmt);
                        global_keys.push(key);
                    }
                    // Files are read synchronously by the provider, no connector needed unless we want to watch them
                }
                SourceConfig::Nacos { server_addr, namespace, username, password, configs } => {
                    let mut connector_configs = Vec::new();
                    let mut connector_keys = Vec::new();
                    
                    for nacos_cfg in configs {
                        let fmt = match nacos_cfg.file_extension.as_str() {
                            "yaml" | "yml" => "yaml",
                            "toml" => "toml",
                            "json" => "json",
                            "ini" => "ini",
                            "properties" => "properties",
                            _ => "yaml",
                        };
                        let uri = format!("nacos://{}/{}/{}", server_addr, nacos_cfg.group, nacos_cfg.data_id);
                        let key = RawItemKey::new(uri, fmt);
                        
                        global_keys.push(key.clone());
                        connector_keys.push(key);
                        connector_configs.push(nacos_cfg.clone());
                    }
                    
                    // Instantiate real Nacos connector unless it was already injected via `add_connector`
                    // We check if ALL keys for this connector are already managed
                    let all_managed = connector_keys.iter().all(|k| self.connectors.iter().any(|c| c.keys().contains(k)));
                    
                    if !all_managed {
                        let connector = NacosConnector::new(
                            server_addr.clone(),
                            namespace.clone().unwrap_or_else(|| "public".to_string()),
                            username.clone(),
                            password.clone(),
                            connector_configs,
                        ).await?;
                        self.connectors.push(Box::new(connector));
                    }
                }
                SourceConfig::Env { prefix } => {
                    let key = RawItemKey::new(format!("env://{}", prefix), "env_kv");
                    global_keys.push(key.clone());
                    
                    if !self.connectors.iter().any(|c| c.keys().contains(&key)) {
                        self.connectors.push(Box::new(EnvConnector {
                            key: key.clone(),
                            mocked_env: None,
                        }));
                    }
                }
                SourceConfig::Args => {
                    let key = RawItemKey::new("args://", "args_kv");
                    global_keys.push(key.clone());
                    
                    if !self.connectors.iter().any(|c| c.keys().contains(&key)) {
                        self.connectors.push(Box::new(ArgsConnector {
                            key: key.clone(),
                            mocked_args: None,
                        }));
                    }
                }
            }
        }

        // Register the topology
        builder = builder.with_global_sources(global_keys);

        // Phase 3: Connect and fetch initial data
        for connector in self.connectors.iter() {
            let initial_data = connector.fetch_initial().await?;
            for (key, content_opt) in initial_data {
                if let Some(content) = content_opt {
                    builder = builder.with_raw_content(key, &content);
                }
            }
        }

        // Phase 4: Build Engine
        let engine = Arc::new(builder.build());

        // Phase 5: Start watchers
        for connector in self.connectors {
            let engine_clone = engine.clone();
            let cb = Box::new(move |key: RawItemKey, new_content: String| {
                engine_clone.update_raw_content(key, new_content);
            });
            connector.watch(cb).await?;
        }

        Ok(engine)
    }
}
