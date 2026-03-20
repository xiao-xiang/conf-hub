pub mod args;
pub mod env;
pub mod mock_nacos;
pub mod nacos;

use crate::error::ConfigError;
use crate::keys::RawItemKey;
use async_trait::async_trait;

#[async_trait]
pub trait SourceConnector: Send + Sync {
    /// Get the initial content synchronously during bootstrap
    async fn fetch_initial(&self) -> Result<std::collections::HashMap<RawItemKey, Option<String>>, ConfigError>;

    /// Initialize the source. If it's a push-based source (like Nacos), start the background listener.
    /// The `on_update` callback should be called whenever new data arrives.
    async fn watch(&self, on_update: Box<dyn Fn(RawItemKey, String) + Send + Sync>) -> Result<(), ConfigError>;
    
    /// Get the target raw item keys this connector corresponds to
    fn keys(&self) -> Vec<RawItemKey>;
}
