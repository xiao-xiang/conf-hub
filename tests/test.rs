
use serde::Deserialize;
use conf_hub::{ConfigEngine, ConfigBind};
use tracing_subscriber::fmt;



#[derive(Debug, Deserialize, PartialEq)]
struct AConfig {
    zhangsan: Z,
    path : String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Z {
    name: String,
    age : usize,
}

impl ConfigBind for AConfig {
    const PATH: Option<&'static str> = None;
}


#[tokio::test]
async fn main() {
    fmt::init();
    
    // 仅仅这一句话，全部搞定！
    let engine = ConfigEngine::from_bootstrap("bootstrap.yaml").await.unwrap();

    let a_config = engine.load::<AConfig>().unwrap();
    println!("{:#?}", a_config.load().zhangsan.name);
    println!("{:#?}", a_config.load().zhangsan.age);
    println!("{:#?}", a_config.load().path);
}
