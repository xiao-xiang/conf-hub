use crate::bootstrap::{BootstrapConfig, SourceConfig};
use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey};
use crate::parsers::get_parser_fn;
use crate::providers::CfgProviders;
use crate::source_manager::args::ArgsProvider;
use crate::source_manager::env::EnvProvider;
use crate::source_manager::file::FileRawProvider;
use crate::source_manager::nacos::NacosRawProvider;
use crate::source_manager::{ConfigNodeProvider, ParserDecorator};
use arc_swap::ArcSwap;
use crossbeam_queue::SegQueue;
use serde::de::DeserializeOwned;
use std::any::TypeId;
use std::collections::HashSet;
use std::sync::{Arc, Weak};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::error;

pub trait ConfigBind: DeserializeOwned + Send + Sync + 'static {
    const PATH: Option<&'static str> = None;

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

const UPDATE_DEBOUNCE_MS: u64 = 50;

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
                    let provider = EnvProvider::new(prefix.clone().unwrap_or_default());
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
        let engine_arc = self.build_arc().await?;
        Arc::try_unwrap(engine_arc)
            .map_err(|_| ConfigError::Provider("ConfigEngine is shared and cannot be moved".to_string()))
    }

    pub async fn build_arc(self) -> Result<Arc<ConfigEngine>, ConfigError> {
        let tcx = Arc::new(CfgCtxt::new(self.providers, self.node_providers, self.global_source_ids));
        // 将无界队列改为有界队列，设置容量为100。
        // 因为我们在worker端有去重（HashSet），所以队列没必要无限长。如果更新风暴超过100，这里我们利用mpsc::channel的特性进行背压
        let (update_tx, update_rx) = mpsc::channel(100);

        let engine = ConfigEngine {
            tcx,
            updaters: SegQueue::new(),
            update_tx,
        };

        let engine_arc = Arc::new(engine);
        ConfigEngine::spawn_update_worker(Arc::downgrade(&engine_arc), update_rx);

        for provider in engine_arc.tcx().node_providers.values() {
            let weak_engine = Arc::downgrade(&engine_arc);
            provider.watch(Arc::new(move |node_id| {
                if let Some(engine) = weak_engine.upgrade() {
                    // enqueue_source_update现在由于是有界队列的try_send，满了的话就忽略这次通知（因为后台正在处理同一批更新）
                    if let Err(err) = engine.enqueue_source_update(node_id) {
                        error!("failed to enqueue source update: {err}");
                    }
                }
            })).await?;
        }

        Ok(engine_arc)
    }
}

struct UpdaterEntry {
    key: TypedNodeKey,
    updater: Box<dyn Fn(&CfgCtxt) -> Result<bool, ConfigError> + Send + Sync>,
}

pub struct ConfigEngine {
    tcx: Arc<CfgCtxt>,
    updaters: SegQueue<UpdaterEntry>,
    update_tx: mpsc::Sender<String>,
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

        let initial_val: Arc<T> = self.tcx.typed_config::<T>(key.clone())?;
        let arc_swap = Arc::new(ArcSwap::new(initial_val));

        let weak_swap = Arc::downgrade(&arc_swap);
        let key_clone = key.clone();
        
        let updater = Box::new(move |tcx: &CfgCtxt| -> Result<bool, ConfigError> {
            if let Some(swap) = weak_swap.upgrade() {
                let new_val = tcx.typed_config::<T>(key_clone.clone())?;
                swap.store(new_val);
                Ok(true)
            } else {
                Ok(false)
            }
        });

        self.updaters.push(UpdaterEntry { key, updater });

        Ok(arc_swap)
    }

    pub fn update_source(&self, node_id: String) {
        if let Err(err) = self.enqueue_source_update(node_id) {
            error!("failed to enqueue source update: {err}");
        }
    }

    fn enqueue_source_update(&self, node_id: String) -> Result<(), ConfigError> {
        // 使用 try_send 进行背压控制。当队列满时，直接丢弃该事件，避免内存溢出。
        // 丢弃是安全的，因为配置引擎在 process_updates 时会全量拉取源最新状态。
        match self.update_tx.try_send(node_id) {
            Ok(_) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // 队列已满，安全丢弃
                Ok(())
            },
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(ConfigError::Provider("update queue is closed".to_string()))
            }
        }
    }

    fn spawn_update_worker(weak_engine: Weak<ConfigEngine>, mut update_rx: mpsc::Receiver<String>) {
        tokio::spawn(async move {
            while let Some(first_node_id) = update_rx.recv().await {
                let mut dirty_nodes = HashSet::new();
                dirty_nodes.insert(first_node_id);

                sleep(Duration::from_millis(UPDATE_DEBOUNCE_MS)).await;
                // 去重合并更新事件
                while let Ok(node_id) = update_rx.try_recv() {
                    dirty_nodes.insert(node_id);
                }

                if let Some(engine) = weak_engine.upgrade() {
                    engine.process_updates(dirty_nodes);
                } else {
                    break;
                }
            }
        });
    }

    fn process_updates(&self, dirty_nodes: HashSet<String>) {
        for node_id in dirty_nodes {
            self.tcx.invalidate_source(node_id);
        }
        self.reload_dirty();
    }

    pub fn reload_dirty(&self) {
        let len = self.updaters.len();
        let mut retained = Vec::with_capacity(len);
        
        // 由于 SegQueue 是无锁队列，我们将其全部 pop 出来处理
        for _ in 0..len {
            if let Some(entry) = self.updaters.pop() {
                if !self.tcx.is_typed_node_dirty(&entry.key) {
                    retained.push(entry);
                    continue;
                }

                match (entry.updater)(&self.tcx) {
                    Ok(true) => retained.push(entry),
                    Ok(false) => {}
                    Err(err) => {
                        error!("failed to reload typed config {}: {err}", entry.key.type_name);
                        retained.push(entry);
                    }
                }
            }
        }

        // 把处理完依然有效的 entry 重新 push 回队列
        // 期间并发 load 新增的 entry 会直接在队列里，不受影响
        for entry in retained {
            self.updaters.push(entry);
        }
    }

    pub fn reload_all(&self) {
        self.force_reload_all();
    }

    pub fn force_reload_all(&self) {
        let len = self.updaters.len();
        let mut retained = Vec::with_capacity(len);
        
        for _ in 0..len {
            if let Some(entry) = self.updaters.pop() {
                match (entry.updater)(&self.tcx) {
                    Ok(true) => retained.push(entry),
                    Ok(false) => {}
                    Err(err) => {
                        error!("failed to reload typed config {}: {err}", entry.key.type_name);
                        retained.push(entry);
                    }
                }
            }
        }

        for entry in retained {
            self.updaters.push(entry);
        }
    }
}
