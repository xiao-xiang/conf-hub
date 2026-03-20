use serde::Deserialize;
use confhub::{BootstrapConfig, Bootstrapper, ConfigBind};
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
    let config = BootstrapConfig::load_from_file("bootstrap.yaml").unwrap();
    let bootstrapper = Bootstrapper::new(config);

    let engine = bootstrapper.bootstrap().await.unwrap();

    let a_config = engine.load::<AConfig>().unwrap();
    assert_eq!(a_config.load().name, "zhangsana");
    assert_eq!(a_config.load().age, 16);

    //loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let server_config = engine.load::<ServerConfig>().unwrap();
        let database_url = server_config.load().database_url.clone();
        println!("database_url: {}", database_url);
    //}
}