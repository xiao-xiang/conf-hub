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

#[async_trait]
pub trait RawProvider: Send + Sync + 'static {
    fn node_id(&self) -> String;
    fn raw_fingerprint(&self) -> Result<u64, ConfigError>;
    fn fetch(&self) -> Result<String, ConfigError>;
    async fn watch(&self, on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError>;
}

pub struct ParserDecorator {
    inner: Box<dyn RawProvider>,
    parse_fn: fn(&str) -> Result<ValueMap, ConfigError>,
}

impl ParserDecorator {
    pub fn new(inner: Box<dyn RawProvider>, parse_fn: fn(&str) -> Result<ValueMap, ConfigError>) -> Self {
        Self { inner, parse_fn }
    }
}

#[async_trait]
impl ConfigNodeProvider for ParserDecorator {
    fn node_id(&self) -> String {
        self.inner.node_id()
    }

    fn raw_fingerprint(&self) -> Result<u64, ConfigError> {
        self.inner.raw_fingerprint()
    }

    fn fetch_and_parse(&self) -> Result<Arc<ValueMap>, ConfigError> {
        let raw_text = self.inner.fetch()?;
        if raw_text.trim().is_empty() {
            return Ok(Arc::new(ValueMap::Object(serde_json::Map::new())));
        }
        let ast = (self.parse_fn)(&raw_text)?;
        Ok(Arc::new(ast))
    }

    async fn watch(&self, on_update: Arc<dyn Fn(String) + Send + Sync>) -> Result<(), ConfigError> {
        self.inner.watch(on_update).await
    }
}
