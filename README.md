# Conf-Hub

A modern, high-performance, and reactive configuration management library for Rust, designed specifically for **highly concurrent async environments** (e.g., Tokio, Axum, Actix-web). 

`conf-hub` uses an **Incremental Dependency Graph** and **Wait-Free (Lock-Free)** reading paths to provide dynamic configuration hot-reloading with zero-overhead read access for your application.

[**中文文档 (README-ZH)**](./README-ZH.md)

---

## 🌟 Core Features

- **🚀 Wait-Free Reads**: Uses `ArcSwap` for configuration reading. Business logic reads configuration in memory without any locks, achieving theoretical maximum multi-core scalability.
- **🧠 Incremental Dependency Graph**: The internal engine uses a Directed Acyclic Graph (DAG) to track configuration dependencies. When a data source changes, it only recalculates and triggers updates for the exact affected structs, rejecting full-scale unbrain reloading.
- **🔄 Hot Reloading**: Native support for Push-based data sources (like Nacos). Configurations are updated asynchronously in background threads, completely decoupled from Web worker threads.
- **🧩 Universal AST & Deep Merge**: Unifies heterogenous formats (`yaml`, `toml`, `json`, `properties`, `env`, `args`) into a single AST (`serde_json::Value`). Supports deep merging across multiple sources.
- **🪄 Placeholder Resolution**: Supports cross-file placeholder interpolation like `${server.port}`, preserving data types perfectly.
- **🛡️ High Concurrency Safe**: Uses `DashMap` for sharded locking and `SegQueue` for lock-free listener registration, eliminating "Stop-The-World" pauses in high-traffic web scenarios.

## 🏗️ Architecture Design

`conf-hub` is heavily inspired by **`rustc`'s Demand-Driven Query System (salsa-like)**. 

Instead of traditional "push-based full recalculation" when a configuration file changes, `conf-hub` uses a **Pull-based memoized query model**:

1. **Dependency Tracking (`DepGraph`)**: Every configuration evaluation (e.g., parsing a file, merging JSONs, resolving placeholders, deserializing a struct) is treated as a "Query Node". During evaluation, the engine dynamically records edges between caller and callee nodes.
2. **Memoization (`CachedResult`)**: The results of these queries are cached using `DashMap` along with a cryptographic `fingerprint` of the data.
3. **Lazy Invalidation**: When a source (like Nacos) pushes an update, the engine simply marks the root node as `Red` (dirty) and propagates an `Unknown` state down the dependency graph. 
4. **Early Cut-off**: When the business logic reads a configuration, the engine traverses back up. If a file was updated but its `fingerprint` (content hash) hasn't changed, the query short-circuits, completely avoiding unnecessary JSON parsing and struct deserialization.

### The Pipeline

1. **Sources Layer**: Multiple providers (File, Nacos, Env, Args) fetch raw text.
2. **AST Layer**: Parsed into unified JSON ASTs.
3. **Merge Layer**: Deep merges all global sources based on their priorities.
4. **Resolve Layer**: Resolves `${...}` placeholders recursively.
5. **Typed Layer**: Maps specific subtrees to your Rust `struct` via `serde`.

The `DepGraph` monitors this entire pipeline. If Nacos pushes a change, `conf-hub` precisely marks the specific path as dirty, recalculates the AST, and atomically hot-swaps the `ArcSwap` pointer for your business logic.

## 📦 Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
conf-hub = "0.1.0"
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
```

## 🚀 Quick Start

### 1. Define your Configuration Structs

Use `serde` to define your structures, and implement the `ConfigBind` trait to tell `conf-hub` which path to map to.

```rust
use serde::Deserialize;
use conf_hub::ConfigBind;

#[derive(Debug, Deserialize, PartialEq)]
struct AppConfig {
    name: String,
    version: String,
}

// Map this struct to the "app" key in the merged configuration
impl ConfigBind for AppConfig {
    const PATH: Option<&'static str> = Some("app");
}

#[derive(Debug, Deserialize, PartialEq)]
struct ServerConfig {
    port: u32,
    url: String, // Might be evaluated from placeholder like "http://localhost:${server.port}"
}

// Map this struct to the "server" key
impl ConfigBind for ServerConfig {
    const PATH: Option<&'static str> = Some("server");
}
```

### 2. Prepare a Bootstrap File

You can define all your configuration sources in a single `bootstrap.yaml`.

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

### 3. Load and Run!

```rust
use conf_hub::ConfigEngine;

#[tokio::main]
async fn main() {
    // 1. Build the engine from bootstrap (Only requires one line!)
    let engine = ConfigEngine::builder()
        .load_from_bootstrap("bootstrap.yaml")
        .await
        .expect("Failed to load bootstrap")
        .build_arc()
        .await
        .expect("Failed to build engine");

    // 2. Load your typed configs. 
    // This returns an Arc<ArcSwap<T>>, which is cheap to clone and pass around.
    let app_config = engine.load::<AppConfig>().unwrap();
    let server_config = engine.load::<ServerConfig>().unwrap();

    // 3. Use it in your high-concurrency web handlers!
    loop {
        // `load()` is Wait-Free! No mutex, no blocking.
        println!("App Name: {}", app_config.load().name);
        println!("Server Port: {}", server_config.load().port);
        
        // If Nacos changes `app.yaml`, the engine will automatically update 
        // the internal pointer. The next `load()` will instantly reflect the new values.
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
```

## 🔌 Supported Providers

- **Local Files**: `.yaml`, `.toml`, `.json`, `.ini`, `.properties`
- **Nacos**: Native Nacos SDK integration with dynamic Push listening.
- **Environment Variables**: Automatic conversion from Env to nested JSON.
- **Command Line Args**: Inject config overrides via CLI.

## 🛡️ Concurrency Model

- **Readers (Web Requests)**: Use `ArcSwap::load()`. Absolutely zero locks. Wait-free.
- **Writers (Config Hot-Reload)**: Uses a dedicated Tokio background worker. Updates are debounced and processed using `DashMap` (sharded locks), ensuring that heavy JSON merges and deserializations never block the global application state or Tokio's scheduler.
