use crate::bootstrap::{BootstrapConfig, SourceConfig};
use crate::error::ConfigError;
use crate::facade::ConfigEngine;
use crate::source_manager::args::ArgsProvider;
use crate::source_manager::env::EnvProvider;
use crate::source_manager::file::FileProvider;
use crate::source_manager::nacos::NacosProvider;
use crate::source_manager::ConfigNodeProvider;
use std::sync::Arc;

pub struct Bootstrapper {
    config: BootstrapConfig,
    providers: Vec<Arc<dyn ConfigNodeProvider>>,
}

impl Bootstrapper {
    pub fn new(config: BootstrapConfig) -> Self {
        Self {
            config,
            providers: Vec::new(),
        }
    }

    /// Add a custom provider (useful for injecting mocks during tests)
    pub fn add_provider(&mut self, provider: Arc<dyn ConfigNodeProvider>) {
        self.providers.push(provider);
    }

    pub async fn bootstrap(self) -> Result<Arc<ConfigEngine>, ConfigError> {
        let mut builder = ConfigEngine::builder();

        // Phase 1 & 2: Parse and build topology, and automatically instantiate providers
        for source_cfg in &self.config.sources {
            match source_cfg {
                SourceConfig::File { configs } => {
                    for file_cfg in configs {
                        let fmt = file_cfg.format.as_deref().unwrap_or("yaml").to_string();
                        let provider = FileProvider::new(file_cfg.path.clone(), fmt);
                        builder = builder.add_provider(Arc::new(provider));
                    }
                }
                SourceConfig::Nacos { server_addr, namespace, username, password, configs } => {
                    for nacos_cfg in configs {
                        let provider = NacosProvider::new(
                            server_addr.clone(),
                            namespace.clone().unwrap_or_else(|| "".to_string()),
                            username.clone(),
                            password.clone(),
                            nacos_cfg.data_id.clone(),
                            nacos_cfg.group.clone(),
                            nacos_cfg.dynamic,
                            nacos_cfg.file_extension.clone(),
                        ).await?;
                        builder = builder.add_provider(Arc::new(provider));
                    }
                }
                SourceConfig::Env { prefix } => {
                    let provider = EnvProvider::new(prefix.clone());
                    builder = builder.add_provider(Arc::new(provider));
                }
                SourceConfig::Args => {
                    let provider = ArgsProvider::new(None);
                    builder = builder.add_provider(Arc::new(provider));
                }
            }
        }

        // Add dynamically injected providers (like mocks)
        for provider in self.providers {
            builder = builder.add_provider(provider);
        }

        let engine = Arc::new(builder.build());

        // Phase 3: Start watchers for all providers that support it
        for provider in engine.tcx().node_providers.values() {
            let engine_clone = engine.clone();
            provider.watch(Arc::new(move |node_id| {
                engine_clone.update_source(node_id);
            })).await?;
        }

        Ok(engine)
    }
}
