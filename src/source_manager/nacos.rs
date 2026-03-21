use crate::error::ConfigError;
use crate::source_manager::RawProvider;
use nacos_sdk::api::config::{ConfigChangeListener, ConfigService, ConfigServiceBuilder};
use nacos_sdk::api::props::ClientProps;
use std::sync::{Arc, RwLock};
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct NacosRawProvider {
    node_id: String,
    data_id: String,
    group: String,
    dynamic: bool,
    config_service: ConfigService,
    cache: Arc<RwLock<String>>,
}

impl NacosRawProvider {
    pub async fn new(
        server_addr: String,
        namespace: String,
        username: Option<String>,
        password: Option<String>,
        data_id: String,
        group: String,
        dynamic: bool,
    ) -> Result<Self, ConfigError> {
        let mut props = ClientProps::new().server_addr(&server_addr).namespace(&namespace);

        if let (Some(u), Some(p)) = (username, password) {
            if !u.is_empty() && !p.is_empty() {
                props = props.auth_username(u).auth_password(p);
            }
        }

        let config_service = ConfigServiceBuilder::new(props)
            .enable_auth_plugin_http()
            .build()
            .await
            .map_err(|e| ConfigError::Provider(format!("Failed to build Nacos ConfigService: {:?}", e)))?;

        // Initial fetch
        let initial_resp = config_service.get_config(data_id.clone(), group.clone()).await
            .map_err(|e| ConfigError::Provider(format!("Failed to fetch Nacos config: {:?}", e)))?;
            
        let initial_content = initial_resp.content().to_string();

        Ok(Self {
            node_id: format!("nacos://{}/{}/{}", server_addr, group, data_id),
            data_id,
            group,
            dynamic,
            config_service,
            cache: Arc::new(RwLock::new(initial_content)),
        })
    }
}

struct InnerListener {
    node_id: String,
    on_update: Arc<dyn Fn(String) + Send + Sync>,
    cache: Arc<RwLock<String>>,
}

impl ConfigChangeListener for InnerListener {
    fn notify(&self, config_resp: nacos_sdk::api::config::ConfigResponse) {
        let content = config_resp.content().to_string();
        {
            let mut cache = self.cache.write().unwrap();
            *cache = content;
        }
        (self.on_update)(self.node_id.clone());
    }
}

#[async_trait]
impl RawProvider for NacosRawProvider {
    fn node_id(&self) -> String {
        self.node_id.clone()
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        let cache = self.cache.read().unwrap();
        let mut hasher = DefaultHasher::new();
        cache.hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn fetch(&self) -> Result<String, ConfigError> {
        let raw_text = {
            let cache = self.cache.read().unwrap();
            cache.clone()
        };
        Ok(raw_text)
    }

    async fn watch(&self, on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        if !self.dynamic {
            return Ok(());
        }

        let listener = Arc::new(InnerListener {
            node_id: self.node_id.clone(),
            on_update,
            cache: self.cache.clone(),
        });
        
        self.config_service.add_listener(self.data_id.clone(), self.group.clone(), listener)
            .await
            .map_err(|e| ConfigError::Provider(format!("Failed to add listener to Nacos: {:?}", e)))?;
            
        Ok(())
    }
}
