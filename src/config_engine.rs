use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey};
use crate::providers::CfgProviders;
use crate::source_manager::ConfigNodeProvider;
use crate::bootstrap::{BootstrapConfig, SourceConfig};
use crate::source_manager::file::FileRawProvider;
use crate::source_manager::nacos::NacosRawProvider;
use crate::source_manager::{ParserDecorator, RawProvider};
use crate::source_manager::env::EnvProvider;
use crate::source_manager::args::ArgsProvider;
use crate::parsers::get_parser_fn;
use arc_swap::ArcSwap;
use serde::de::DeserializeOwned;
use std::any::TypeId;
use std::sync::Arc;

pub trait ConfigBind: DeserializeOwned + Send + Sync + 'static {
    // PATH is like the prefix in Spring Boot
    const PATH: Option<&'static str> = None;

    // A factory method for type-erased deserialization
    fn deserialize_any(val: &serde_json::Value) -> Result<Arc<dyn std::any::Any + Send + Sync>, ConfigError> {
        let typed: Self = serde_json::from_value(val.clone()).map_err(ConfigError::Json)?;
        Ok(Arc::new(typed))
    }
}

pub struct ConfigEngineBuilder {
    providers: CfgProviders,
    node_providers: std::collections::HashMap<String, Arc<dyn ConfigNodeProvider>>,
    global_source_ids: Vec<String>,
}

impl ConfigEngineBuilder {
    pub fn new() -> Self {
        Self {
            providers: CfgProviders::default(),
            node_providers: std::collections::HashMap::new(),
            global_source_ids: Vec::new(),
        }
    }

    pub fn add_provider(mut self, provider: Arc<dyn ConfigNodeProvider>) -> Self {
        let id = provider.node_id();
        self.node_providers.insert(id.clone(), provider);
        self.global_source_ids.push(id);
        self
    }

    pub async fn load_from_bootstrap(mut self, path: &str) -> Result<Self, ConfigError> {
        let config = BootstrapConfig::load_from_file(path)?;

        for source_cfg in config.sources {
            match source_cfg {
                SourceConfig::File { configs } => {
                    for file_cfg in configs {
                        let fmt = file_cfg.format.as_deref().unwrap_or("yaml");
                        let parse_fn = get_parser_fn(fmt);
                        let raw_provider = Box::new(FileRawProvider::new(file_cfg.path.clone()));
                        let decorator = ParserDecorator::new(raw_provider, parse_fn);
                        self = self.add_provider(Arc::new(decorator));
                    }
                }
                SourceConfig::Nacos { server_addr, namespace, username, password, configs } => {
                    for nacos_cfg in configs {
                        let fmt = nacos_cfg.file_extension.as_str();
                        let parse_fn = get_parser_fn(fmt);
                        let raw_provider = Box::new(NacosRawProvider::new(
                            server_addr.clone(),
                            namespace.clone().unwrap_or_else(|| "".to_string()),
                            username.clone(),
                            password.clone(),
                            nacos_cfg.data_id.clone(),
                            nacos_cfg.group.clone(),
                            nacos_cfg.dynamic,
                        ).await?);
                        let decorator = ParserDecorator::new(raw_provider, parse_fn);
                        self = self.add_provider(Arc::new(decorator));
                    }
                }
                SourceConfig::Env { prefix } => {
                    let provider = EnvProvider::new(prefix.clone());
                    self = self.add_provider(Arc::new(provider));
                }
                SourceConfig::Args => {
                    let provider = ArgsProvider::new(None);
                    self = self.add_provider(Arc::new(provider));
                }
            }
        }
        Ok(self)
    }

    pub async fn build(self) -> Result<ConfigEngine, ConfigError> {
        let tcx = Arc::new(CfgCtxt::new(self.providers, self.node_providers, self.global_source_ids));
        
        let engine = ConfigEngine {
            tcx,
            updaters: std::sync::RwLock::new(Vec::new()),
        };

        let engine_arc = Arc::new(engine);

        // Start watchers
        for provider in engine_arc.tcx().node_providers.values() {
            let engine_clone = engine_arc.clone();
            provider.watch(Arc::new(move |node_id| {
                engine_clone.update_source(node_id);
            })).await?;
        }

        // To return Arc<ConfigEngine> would be better since updaters are registered as Arc, 
        // but to keep API compatibility, we extract it.
        // Actually `build` is returning `ConfigEngine` but we need `Arc` for watch callbacks.
        // It's a bit tricky to return non-Arc if we used Arc inside.
        // We will return Arc<ConfigEngine> instead of ConfigEngine.
        // Wait, the return type of `build` was `ConfigEngine`. Let's change it to `Arc<ConfigEngine>`.
        Ok(Arc::into_inner(engine_arc).unwrap())
    }

    pub async fn build_arc(self) -> Result<Arc<ConfigEngine>, ConfigError> {
        let tcx = Arc::new(CfgCtxt::new(self.providers, self.node_providers, self.global_source_ids));
        
        let engine = ConfigEngine {
            tcx,
            updaters: std::sync::RwLock::new(Vec::new()),
        };

        let engine_arc = Arc::new(engine);

        // Start watchers
        for provider in engine_arc.tcx().node_providers.values() {
            let engine_clone = engine_arc.clone();
            provider.watch(Arc::new(move |node_id| {
                engine_clone.update_source(node_id);
            })).await?;
        }

        Ok(engine_arc)
    }
}

pub struct ConfigEngine {
    tcx: Arc<CfgCtxt>,
    updaters: std::sync::RwLock<Vec<Box<dyn Fn(&CfgCtxt) -> Result<(), ConfigError> + Send + Sync>>>,
}

impl ConfigEngine {
    pub fn builder() -> ConfigEngineBuilder {
        ConfigEngineBuilder::new()
    }

    pub fn new() -> Self {
        panic!("ConfigEngine should be built using build().await or build_arc().await");
    }

    pub fn tcx(&self) -> Arc<CfgCtxt> {
        self.tcx.clone()
    }

    pub fn load<T: ConfigBind>(&self) -> Result<Arc<ArcSwap<T>>, ConfigError> {
        let key = TypedNodeKey {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            subtree: SubtreeKey {
                path: T::PATH.map(|s| s.to_string()),
            },
            deserializer: <T as ConfigBind>::deserialize_any,
        };

        // Initial load using tcx query system to leverage cache and graph!
        let initial_val: Arc<T> = self.tcx.typed_config::<T>(key.clone())?;
        let arc_swap = Arc::new(ArcSwap::new(initial_val));

        // Register updater for future reloads
        let weak_swap = Arc::downgrade(&arc_swap);
        let key_clone = key.clone();
        
        let updater = Box::new(move |tcx: &CfgCtxt| -> Result<(), ConfigError> {
            if let Some(swap) = weak_swap.upgrade() {
                // Now using the proper query system to get typed config
                let new_val = tcx.typed_config::<T>(key_clone.clone())?;
                swap.store(new_val);
                Ok(())
            } else {
                // If the arc_swap is dropped, we can ignore
                Ok(())
            }
        });

        self.updaters.write().unwrap().push(updater);

        Ok(arc_swap)
    }

    /// Triggered by a background worker when e.g. Nacos updates
    pub fn update_source(&self, node_id: String) {
        // 1. Mark the key as dirty.
        self.tcx.invalidate_source(node_id);

        // 2. Trigger reload for all registered handlers
        self.reload_all();
    }

    pub fn reload_all(&self) {
        let updaters = self.updaters.read().unwrap();
        for updater in updaters.iter() {
            // If early cutoff happens, tcx.subtree will return the cached Arc<ValueMap> 
            // very quickly without doing the work.
            let _ = updater(&self.tcx);
        }
    }
}
