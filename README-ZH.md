# Conf-Hub

一个为 Rust 打造的现代化、高性能、响应式配置管理库，专为 **高并发异步环境**（如 Tokio, Axum, Actix-web）设计。

`conf-hub` 使用了 **增量依赖图 (Incremental Dependency Graph)** 和 **无锁 (Wait-Free)** 的读取路径，为你的应用提供动态配置热重载能力，同时保证业务读取配置的零开销。

[**English Documentation (README)**](./README.md)

---

## 🌟 核心特性

- **🚀 无锁极速读取**: 使用 `ArcSwap` 进行配置读取。业务逻辑在内存中读取配置没有任何锁争用，达到理论上多核扩展的极限。
- **🧠 增量依赖图计算**: 内部引擎使用有向无环图 (DAG) 跟踪配置依赖。当数据源发生变化时，它只精确重算并触发受影响结构体的更新，拒绝无脑全量反序列化。
- **🔄 真正的热重载**: 原生支持基于 Push 的数据源（如 Nacos）。配置更新在后台异步线程完成，与 Web 工作线程完全解耦。
- **🧩 统一 AST 与深度合并**: 将异构数据格式（`yaml`, `toml`, `json`, `properties`, `env`, `args`）统一解析为单一的 AST（`serde_json::Value`），支持跨多个数据源的深度合并。
- **🪄 占位符解析**: 支持跨文件的占位符替换（如 `${server.port}`），完美保留底层数据类型（数字、布尔值等）。
- **🛡️ 高并发安全**: 内部使用 `DashMap` 实现分片锁，使用 `SegQueue` 实现无锁监听器注册，彻底消除高流量 Web 场景下的 "Stop-The-World" 停顿。

## 🏗️ 架构设计

`conf-hub` 的核心设计深度借鉴了 **`rustc` 的需求驱动查询系统 (Demand-Driven Query System，类似 salsa)**。

不同于传统的“配置一变就全量无脑反序列化”的 Push 模型，`conf-hub` 采用的是**带有记忆化缓存的 Pull 模型**：

1. **依赖追踪 (`DepGraph`)**: 每一次配置计算（例如解析文本、合并 JSON、替换占位符、反序列化结构体）都被抽象为一个“查询节点 (Query Node)”。在执行计算时，引擎会动态记录调用者与被调用者之间的图依赖边。
2. **记忆化缓存 (`CachedResult`)**: 这些查询的结果会连同数据的 `fingerprint` (内容指纹) 一起被缓存到高并发的 `DashMap` 中。
3. **惰性失效 (Lazy Invalidation)**: 当底层数据源（如 Nacos）推送更新时，引擎仅仅是将图的根节点标记为 `Red` (脏)，并顺着依赖图将下游节点标记为 `Unknown`。
4. **提前截断 (Early Cut-off)**: 当后台 worker 重算配置时，会沿着依赖树往回查。如果某个文件触发了更新事件，但解析后的 AST `fingerprint`（内容哈希）并未发生改变，查询就会立刻“截断”，彻底避免下游无关配置的不必要重算与反序列化开销。

### 数据流水线

1. **数据源层 (Sources)**: 多种提供者（文件、Nacos、环境变量、命令行参数）抓取原始文本。
2. **AST 层**: 将文本解析为统一的 JSON AST。
3. **合并层 (Merge)**: 根据优先级对所有全局数据源进行深度合并 (Deep Merge)。
4. **解析层 (Resolve)**: 递归解析并替换 `${...}` 占位符。
5. **类型层 (Typed)**: 通过 `serde` 将特定的子树映射到你的 Rust `struct`。

`DepGraph` 监控整个流水线。如果 Nacos 推送了变更，`conf-hub` 会精确标记特定的路径为脏状态，重新计算 AST，并原子化地为你的业务逻辑热替换 `ArcSwap` 指针。

## 📦 安装

在 `Cargo.toml` 中添加依赖:

```toml
[dependencies]
conf-hub = "0.1.0"
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
```

## 🚀 快速开始

### 1. 定义你的配置结构体

使用 `serde` 定义结构体，并实现 `ConfigBind` trait 来告诉 `conf-hub` 这个结构体应该映射到配置的哪个路径。

```rust
use serde::Deserialize;
use conf_hub::ConfigBind;

#[derive(Debug, Deserialize, PartialEq)]
struct AppConfig {
    name: String,
    version: String,
}

// 将此结构体映射到合并后配置的 "app" 键
impl ConfigBind for AppConfig {
    const PATH: Option<&'static str> = Some("app");
}

#[derive(Debug, Deserialize, PartialEq)]
struct ServerConfig {
    port: u32,
    url: String, // 可以通过占位符求值，例如 "http://localhost:${server.port}"
}

// 将此结构体映射到 "server" 键
impl ConfigBind for ServerConfig {
    const PATH: Option<&'static str> = Some("server");
}
```

### 2. 准备启动配置文件

你可以在一个单一的 `bootstrap.yaml` 中定义所有的配置数据源。

```yaml
# bootstrap.yaml
sources:
  - type: nacos
    server_addr: "127.0.0.1:8848"
    namespace: "public"
    configs:
      - data_id: "app.yaml"
        group: "DEFAULT_GROUP"
        dynamic: true
        file_extension: "yaml"

  - type: file
    configs:
      - path: "server.toml"
        format: "toml"
        
  - type: env
    prefix: "MY_APP_"
```

### 3. 加载并运行！

```rust
use conf_hub::ConfigEngine;

#[tokio::main]
async fn main() {
    // 1. 从 bootstrap 构建引擎（只需要一行代码！）
    let engine = ConfigEngine::builder()
        .load_from_bootstrap("bootstrap.yaml")
        .await
        .expect("Failed to load bootstrap")
        .build_arc()
        .await
        .expect("Failed to build engine");

    // 2. 加载你的强类型配置。
    // 这会返回一个 Arc<ArcSwap<T>>，它的 clone 成本极低，可以方便地在不同模块间传递。
    let app_config = engine.load::<AppConfig>().unwrap();
    let server_config = engine.load::<ServerConfig>().unwrap();

    // 3. 在你的高并发 Web handler 中使用它！
    loop {
        // load() 是 Wait-Free (无锁) 的！没有互斥锁，不会阻塞。
        println!("App Name: {}", app_config.load().name);
        println!("Server Port: {}", server_config.load().port);
        
        // 如果 Nacos 更改了 `app.yaml`，引擎会自动更新内部指针。
        // 下一次调用 load() 时将立即反映新的值。
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
```

## 🔌 支持的配置源

- **本地文件**: `.yaml`, `.toml`, `.json`, `.ini`, `.properties`
- **Nacos**: 集成原生 Nacos SDK，支持动态 Push 监听。
- **环境变量**: 自动将环境变量转换为嵌套的 JSON 结构。
- **命令行参数**: 支持通过 CLI 注入配置覆盖。

## 🛡️ 并发模型设计

- **读端 (Web 请求)**: 业务侧使用 `ArcSwap::load()` 获取配置。完全零锁（Wait-free）。
- **写端 (配置热更新)**: 使用专门的 Tokio 后台 worker。更新事件经过防抖去重处理，内部图计算使用 `DashMap`（分片锁），确保繁重的 JSON 合并和反序列化操作永远不会阻塞全局应用状态或饿死 Tokio 的调度器。
