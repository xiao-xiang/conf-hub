use confhub::facade::ConfigEngineBuilder;
use confhub::keys::RawItemKey;

#[test]
fn test_placeholder() {
    let engine = ConfigEngineBuilder::new()
        .with_global_sources(vec![
            RawItemKey {
                uri: "memory://config.json".to_string(),
                parser_type: "json".to_string(),
            }
        ])
        .with_raw_content(
            RawItemKey {
                uri: "memory://config.json".to_string(),
                parser_type: "json".to_string(),
            },
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
            }"#
        )
        .build();

    let root = engine.tcx().resolved_global().unwrap();
    println!("Resolved: {}", serde_json::to_string_pretty(&*root).unwrap());
}
