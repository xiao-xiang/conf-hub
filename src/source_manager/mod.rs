pub mod args;
pub mod env;
pub mod file;
pub mod mock_nacos;
pub mod nacos;

use crate::error::ConfigError;
use serde_json::Value as ValueMap;
use std::sync::Arc;
use async_trait::async_trait;

#[async_trait]
pub trait ConfigNodeProvider: Send + Sync + 'static {
    /// 节点的唯一标识（用于图缓存）
    fn node_id(&self) -> String;
    
    /// 获取指纹，用于提早截断
    fn raw_fingerprint(&self) -> Result<u64, ConfigError>;
    
    /// 核心！拉取并直接返回解析好的 AST
    fn fetch_and_parse(&self) -> Result<Arc<ValueMap>, ConfigError>;
    
    /// 如果是 push-based 的源，可以在这里注册监听器
    async fn watch(&self, on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError>;
}
