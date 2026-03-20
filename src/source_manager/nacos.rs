use crate::bootstrap::NacosConfigItem;
use crate::error::ConfigError;
use crate::keys::RawItemKey;
use crate::source_manager::SourceConnector;
use nacos_sdk::api::config::{ConfigChangeListener, ConfigService, ConfigServiceBuilder};
use nacos_sdk::api::props::ClientProps;
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;

pub struct NacosConnector {
    pub server_addr: String,
    pub namespace: String,
    pub configs: Vec<NacosConfigItem>,
    config_service: ConfigService,
}

impl NacosConnector {
    pub async fn new(
        server_addr: String,
        namespace: String,
        username: Option<String>,
        password: Option<String>,
        configs: Vec<NacosConfigItem>,
    ) -> Result<Self, ConfigError> {
        let mut props = ClientProps::new().server_addr(&server_addr).namespace(&namespace);

        if let (Some(u), Some(p)) = (username.clone(), password.clone()) {
            // Check if username/password are actually empty strings from yaml parsing
            if !u.is_empty() && !p.is_empty() {
                props = props.auth_username(u).auth_password(p);
            }
        }

        // Nacos Rust SDK auth handling logic requires auth properties correctly passed
        // For Nacos SDK > 0.3.0, it might need to build within a block_on if we don't pass an existing runtime
        // But since we are already in an async context, we can just await it directly!
        let config_service = ConfigServiceBuilder::new(props)
            .enable_auth_plugin_http()
            .build()
            .await
            .map_err(|e| ConfigError::Provider(format!("Failed to build Nacos ConfigService: {:?}", e)))?;

        Ok(Self {
            server_addr,
            namespace,
            configs,
            config_service,
        })
    }
}

struct InnerListener {
    key: RawItemKey,
    on_update: Arc<Box<dyn Fn(RawItemKey, String) + Send + Sync>>,
}

impl ConfigChangeListener for InnerListener {
    fn notify(&self, config_resp: nacos_sdk::api::config::ConfigResponse) {
        let content = config_resp.content().to_string();
        (self.on_update)(self.key.clone(), content);
    }
}

#[async_trait]
impl SourceConnector for NacosConnector {
    async fn fetch_initial(&self) -> Result<HashMap<RawItemKey, Option<String>>, ConfigError> {
        let mut results = HashMap::new();
        
        for item in &self.configs {
            let key = RawItemKey::new(
                format!("nacos://{}/{}/{}", self.server_addr, item.group, item.data_id),
                item.file_extension.clone(),
            );
            
            let resp_result = self.config_service.get_config(item.data_id.clone(), item.group.clone()).await;
            
            match resp_result {
                Ok(resp) => {
                    results.insert(key, Some(resp.content().to_string()));
                },
                Err(e) => {
                    println!("Warning: Failed to fetch config {} from Nacos: {:?}", item.data_id, e);
                    // Depending on strictness, we might want to fail the whole bootstrap or just skip it
                    // For now, we will skip it to allow the application to start if some configs are missing or auth fails
                    // In a production app, you might want to configure this behavior (fail-fast vs warn)
                    results.insert(key, None);
                }
            }
        }
        
        Ok(results)
    }

    async fn watch(&self, on_update: Box<dyn Fn(RawItemKey, String) + Send + Sync>) -> Result<(), ConfigError> {
        let shared_cb = Arc::new(on_update);
        
        for item in &self.configs {
            if !item.dynamic {
                continue;
            }
            
            let key = RawItemKey::new(
                format!("nacos://{}/{}/{}", self.server_addr, item.group, item.data_id),
                item.file_extension.clone(),
            );
            
            let listener = Arc::new(InnerListener {
                key,
                on_update: shared_cb.clone(),
            });
            
            self.config_service.add_listener(item.data_id.clone(), item.group.clone(), listener)
                .await
                .map_err(|e| ConfigError::Provider(format!("Failed to add listener to Nacos: {:?}", e)))?;
        }
            
        Ok(())
    }

    fn keys(&self) -> Vec<RawItemKey> {
        self.configs.iter().map(|item| {
            RawItemKey::new(
                format!("nacos://{}/{}/{}", self.server_addr, item.group, item.data_id),
                item.file_extension.clone(),
            )
        }).collect()
    }
}
