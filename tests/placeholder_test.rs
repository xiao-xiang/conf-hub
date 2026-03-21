use confhub::facade::ConfigEngineBuilder;
use confhub::source_manager::mock_nacos::MockNacosRawProvider;
use confhub::source_manager::ParserDecorator;
use std::sync::Arc;

#[tokio::test]
async fn test_placeholder() {
    let raw_provider = Box::new(MockNacosRawProvider::new(
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
        vec![]
    ));

    let parse_fn = confhub::parsers::get_parser_fn("json");
    let provider = Arc::new(ParserDecorator::new(raw_provider, parse_fn));

    let engine = ConfigEngineBuilder::new()
        .add_provider(provider)
        .build()
        .await
        .unwrap();

    let root = engine.tcx().resolved_global().unwrap();
    println!("Resolved: {}", serde_json::to_string_pretty(&*root).unwrap());
}
