use confhub::facade::ConfigEngineBuilder;
use confhub::source_manager::mock_nacos::MockNacosProvider;
use std::sync::Arc;

#[test]
fn test_placeholder() {
    let provider = Arc::new(MockNacosProvider::new(
        "memory://config.json".to_string(),
        r#"{
            "app": {
                "name": "my-app",
                "greeting": "Hello, ${app.name}!",
                "nested": "${app.greeting} Welcome to ${server.port}"
            },
            "server": {
                "port": 8080,
                "url": "http://localhost:${server.port}"
            }
        }"#.to_string(),
        "json".to_string(),
        vec![]
    ));

    let engine = ConfigEngineBuilder::new()
        .add_provider(provider)
        .build();

    let root = engine.tcx().resolved_global().unwrap();
    println!("Resolved: {}", serde_json::to_string_pretty(&*root).unwrap());
}
