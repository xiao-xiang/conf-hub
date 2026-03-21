use serde::Deserialize;
use conf_hub::{ConfigEngine, ConfigBind};
use tracing_subscriber::fmt;



#[derive(Debug, Deserialize, PartialEq)]
struct AConfig {
    name: String,
    age: u32,
}

impl ConfigBind for AConfig {
    const PATH: Option<&'static str> = Some("a");
}


#[tokio::test]
async fn main() {
    fmt::init();
    
    // 仅仅这一句话，全部搞定！
    let engine = ConfigEngine::builder()
        .load_from_bootstrap("bootstrap.yaml")
        .await
        .unwrap()
        .build_arc()
        .await
        .unwrap();

    let a_config = engine.load::<AConfig>().unwrap();
    println!("{:#?}", a_config.load().name);
}
