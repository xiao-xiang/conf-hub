pub mod context;
pub mod dep_graph;
pub mod error;
pub mod facade;
pub mod keys;
pub mod providers;

pub use context::CfgCtxt;
pub use error::ConfigError;
pub use facade::{ConfigBind, ConfigEngine};
pub use keys::{Format, RawItemKey, SourceKey, SubtreeKey, TypedNodeKey};
pub use providers::CfgProviders;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::fs;

    #[derive(Debug, Deserialize, PartialEq)]
    struct AppConfig {
        name: String,
        port: u16,
    }

    impl ConfigBind for AppConfig {
        const SOURCE: &'static str = "app_config";
        const PATH: Option<&'static str> = None;
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct DbConfig {
        url: String,
    }

    impl ConfigBind for DbConfig {
        const SOURCE: &'static str = "app_config";
        const PATH: Option<&'static str> = Some("database");
    }

    #[test]
    fn test_comprehensive_config_engine() {
        // 1. Prepare some mock files
        let dir = std::env::temp_dir().join("confhub_test");
        fs::create_dir_all(&dir).unwrap();
        
        let yaml_path = dir.join("app.yaml");
        fs::write(&yaml_path, "name: yaml_app\ndatabase:\n  url: mysql://yaml\n").unwrap();
        
        let ini_path = dir.join("app.ini");
        fs::write(&ini_path, "port=8080\n").unwrap();
        
        let nacos_key = RawItemKey::Nacos {
            group: "DEFAULT".to_string(),
            data_id: "app.json".to_string(),
            format: Format::Json,
        };
        let nacos_content = r#"{"database": {"url": "mysql://${name}:${port}"}}"#;

        let env_key = RawItemKey::Env("APP_".to_string());
        // mock env by injecting into raw store
        let env_content = r#"{"APP_NAME": "env_app"}"#;

        let args_key = RawItemKey::Args;
        // mock args
        let args_content = r#"["--port=9090"]"#;

        // Priority: Yaml < Ini < Nacos < Env < Args
        // The list is in order of application (later ones override earlier ones)
        let engine = ConfigEngine::builder()
            .register_source("app_config", vec![
                RawItemKey::File(yaml_path.to_str().unwrap().to_string(), Format::Yaml),
                RawItemKey::File(ini_path.to_str().unwrap().to_string(), Format::Ini),
                nacos_key.clone(),
                env_key.clone(),
                args_key.clone(),
            ])
            .with_raw_content(nacos_key.clone(), nacos_content)
            .with_raw_content(env_key.clone(), env_content)
            .with_raw_content(args_key.clone(), args_content)
            .build();
            
        // Let's print the merged map to see what we are trying to deserialize
        let app_cfg = engine.load::<AppConfig>().unwrap();
        let db_cfg = engine.load::<DbConfig>().unwrap();

        // 1. Initial assertion
        // name: from Yaml(yaml_app) -> overridden by Env(env_app)
        assert_eq!(app_cfg.load().name, "env_app");
        // port: from Ini(8080) -> overridden by Args(9090)
        assert_eq!(app_cfg.load().port, 9090);
        // db url: from Yaml -> Nacos placeholder
        // placeholder resolves to "mysql://env_app:9090"
        assert_eq!(db_cfg.load().url, "mysql://env_app:9090");

        // 2. Real-time Update (Nacos)
        engine.update_raw_content(nacos_key, r#"{"database": {"url": "postgresql://${name}"}}"#.to_string());
        
        // db url should be updated, app_cfg should remain exactly the same without re-deserializing if we check fingerprints!
        assert_eq!(db_cfg.load().url, "postgresql://env_app");
        assert_eq!(app_cfg.load().name, "env_app");
    }
}
