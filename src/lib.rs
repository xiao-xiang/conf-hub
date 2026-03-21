pub mod bootstrap;
pub mod context;
pub mod dep_graph;
pub mod error;
pub mod config_engine;
pub mod keys;
pub mod providers;
pub mod source_manager;
pub mod parsers;

pub use bootstrap::{BootstrapConfig, SourceConfig};
pub use context::CfgCtxt;
pub use error::ConfigError;
pub use config_engine::{ConfigBind, ConfigEngine, ConfigEngineBuilder};
pub use keys::{SubtreeKey, TypedNodeKey};
pub use providers::CfgProviders;
pub use source_manager::ConfigNodeProvider;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct AConfig {
        name: String,
        age: u32,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct ServerConfig {
        grpc_port: u32,
        http_port: u32,
        rabbitmq_url: String,
        test_test: u32, // 这里定义为整型，验证占位符是否保留了数字类型
    }

    impl ConfigBind for AConfig {
        const PATH: Option<&'static str> = Some("a");
    }

    impl ConfigBind for ServerConfig {
        const PATH: Option<&'static str> = Some("server");
    }

    impl ConfigBind for serde_json::Value {
        const PATH: Option<&'static str> = None;
    }

    #[tokio::test]
    async fn test_bootstrapper_auto_wire() {
        // Just one line to load everything!
        let engine = ConfigEngine::builder()
            .load_from_bootstrap("bootstrap.yaml")
            .await
            .unwrap()
            .build_arc()
            .await
            .unwrap();

        // 3. Load the generic Value directly to assert it works without knowing the struct
        let root_config = engine.load::<serde_json::Value>().unwrap();
        println!("Root config from ArcSwap: {:?}", serde_json::to_string(&*(root_config.load().clone())).unwrap());
        
        // Assert that the config is not null (meaning it successfully fetched and merged something)
        assert!(!root_config.load().is_null());

        // 4. Test mapping to a specific struct based on the real output we saw
        let a_config = engine.load::<AConfig>().unwrap();
        assert_eq!(a_config.load().name, "zhangsana");
        assert_eq!(a_config.load().age, 16);

        // 5. Test type-preserving placeholder replacement
        let server_config = engine.load::<ServerConfig>().unwrap();
        assert_eq!(server_config.load().test_test, 16); // test_test is perfectly loaded as u32
    }
}

