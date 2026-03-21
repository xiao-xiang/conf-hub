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


#[derive(Debug, Deserialize, PartialEq)]
struct ServerConfig {
    database_url: String, // 这里定义为整型，验证占位符是否保留了数字类型
    redis_url: String, // 这里定义为整型，验证占位符是否保留了数字类型
    bbb: String, // 这里定义为整型，验证占位符是否保留了数字类型
}
impl ConfigBind for ServerConfig {
    const PATH: Option<&'static str> = None;
}



#[tokio::main]
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
    assert_eq!(a_config.load().name, "zhangsana");
    assert_eq!(a_config.load().age, 16);

    let server_config = engine.load::<ServerConfig>().unwrap();
    let database_url = server_config.load().database_url.clone();
    println!("database_url: {}", database_url);
}