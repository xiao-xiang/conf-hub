use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{Format, RawItemKey, SourceKey, SubtreeKey, TypedNodeKey};
use serde_json::Value as ValueMap;
use std::any::Any;
use std::fs;
use std::sync::Arc;
use ini::Ini;
use java_properties::read;
use std::collections::HashMap;
use regex::Regex;
use lazy_static::lazy_static;

pub struct CfgProviders {
    pub raw_item: fn(&CfgCtxt, RawItemKey) -> Result<Arc<String>, ConfigError>,
    pub parsed_item: fn(&CfgCtxt, RawItemKey) -> Result<Arc<ValueMap>, ConfigError>,
    pub merged_source: fn(&CfgCtxt, SourceKey) -> Result<Arc<ValueMap>, ConfigError>,
    pub resolved_source: fn(&CfgCtxt, SourceKey) -> Result<Arc<ValueMap>, ConfigError>,
    pub subtree: fn(&CfgCtxt, SubtreeKey) -> Result<Arc<ValueMap>, ConfigError>,
    pub typed_config: fn(&CfgCtxt, TypedNodeKey) -> Result<Arc<dyn Any + Send + Sync>, ConfigError>,
    
    // Extensibility for custom deserialization logic
    pub deserializer: fn(&ValueMap, &TypedNodeKey) -> Result<Arc<dyn Any + Send + Sync>, ConfigError>,
}

impl Default for CfgProviders {
    fn default() -> Self {
        Self {
            raw_item: default_raw_item,
            parsed_item: default_parsed_item,
            merged_source: default_merged_source,
            resolved_source: default_resolved_source,
            subtree: default_subtree,
            typed_config: default_typed_config,
            deserializer: |_, _| Err(ConfigError::Provider("Deserializer not configured for type".into())),
        }
    }
}

fn default_raw_item(tcx: &CfgCtxt, key: RawItemKey) -> Result<Arc<String>, ConfigError> {
    match key {
        RawItemKey::File(ref path, _) => {
            let content = fs::read_to_string(path).map_err(ConfigError::Io)?;
            Ok(Arc::new(content))
        }
        RawItemKey::Nacos { .. } | RawItemKey::Env(_) | RawItemKey::Args => {
            // Read from dynamic raw store
            let store = tcx.raw_store.read().unwrap();
            if let Some(content) = store.get(&key) {
                Ok(Arc::new(content.clone()))
            } else {
                Ok(Arc::new(String::new())) // Return empty string if not found/initialized
            }
        }
    }
}

fn insert_nested(root: ValueMap, path: &str, val: &str) -> ValueMap {
    let parts: Vec<&str> = path.split('.').collect();
    
    // We will build the tree iteratively and then merge it back or just construct the path.
    // Building it recursively is simpler in Rust to avoid lifetime issues.
    
    fn build_tree(parts: &[&str], val: &str) -> ValueMap {
        if parts.is_empty() {
            return ValueMap::Null;
        }
        if parts.len() == 1 {
            let parsed_val = if let Ok(b) = val.parse::<bool>() {
                ValueMap::Bool(b)
            } else if let Ok(n) = val.parse::<u64>() {
                ValueMap::Number(n.into())
            } else if let Ok(n) = val.parse::<i64>() {
                ValueMap::Number(n.into())
            } else if let Ok(n) = val.parse::<f64>() {
                serde_json::Number::from_f64(n).map(ValueMap::Number).unwrap_or_else(|| ValueMap::String(val.to_string()))
            } else {
                ValueMap::String(val.to_string())
            };
            
            let mut map = serde_json::Map::new();
            map.insert(parts[0].to_string(), parsed_val);
            return ValueMap::Object(map);
        }
        
        let mut map = serde_json::Map::new();
        map.insert(parts[0].to_string(), build_tree(&parts[1..], val));
        ValueMap::Object(map)
    }
    
    // Only process if parts is valid
    if parts.is_empty() || parts[0].is_empty() {
        return root;
    }
    
    let new_tree = build_tree(&parts, val);
    let mut cloned_root = root.clone();
    
    // If the root is somehow a scalar, we should reset it to Object to merge into it.
    if !cloned_root.is_object() {
        cloned_root = ValueMap::Object(serde_json::Map::new());
    }
    
    deep_merge_values(&mut cloned_root, &new_tree);
    cloned_root
}

fn deep_merge_values(target: &mut ValueMap, source: &ValueMap) {
    match (target, source) {
        (ValueMap::Object(a), ValueMap::Object(b)) => {
            for (k, v) in b {
                let entry = a.entry(k.clone()).or_insert(ValueMap::Null);
                // If the existing entry is not an object, but we are merging an object into it,
                // we should overwrite it with an empty object first so we can deep merge into it.
                if !entry.is_object() && v.is_object() {
                    *entry = ValueMap::Object(serde_json::Map::new());
                }
                deep_merge_values(entry, v);
            }
        }
        (a, b) => {
            if !b.is_null() {
                *a = b.clone();
            }
        }
    }
}

fn parse_ini(text: &str) -> Result<ValueMap, ConfigError> {
    let mut root = ValueMap::Object(serde_json::Map::new());
    // Use load_from_str instead of load_from_file
    let ini = Ini::load_from_str(text).map_err(|e| ConfigError::Provider(format!("INI parse error: {}", e)))?;
    
    for (sec, prop) in ini.iter() {
        let section_name = sec.unwrap_or("");
        for (k, v) in prop.iter() {
            let path = if section_name.is_empty() {
                k.to_string()
            } else {
                format!("{}.{}", section_name, k)
            };
            // Here root is passed by value and returned, must re-assign to root
            root = insert_nested(root, &path, v);
        }
    }
    Ok(root)
}

fn parse_properties(text: &str) -> Result<ValueMap, ConfigError> {
    let mut root = ValueMap::Object(serde_json::Map::new());
    let props = read(text.as_bytes()).map_err(|e| ConfigError::Provider(format!("Properties parse error: {}", e)))?;
    
    for (k, v) in props {
        root = insert_nested(root, &k, &v);
    }
    Ok(root)
}

fn parse_env(text: &str, prefix: &str) -> ValueMap {
    let mut root = ValueMap::Object(serde_json::Map::new());
    if let Ok(env_map) = serde_json::from_str::<HashMap<String, String>>(text) {
        for (k, v) in env_map {
            if k.starts_with(prefix) {
                let key_without_prefix = &k[prefix.len()..];
                let path = key_without_prefix.replace("__", ".").to_lowercase();
                root = insert_nested(root, &path, &v);
            } else {
                // If prefix is not matched exactly but maybe we just want to match it directly
                // (e.g. for testing)
            }
        }
    }
    root
}

fn parse_args(text: &str) -> ValueMap {
    let mut root = ValueMap::Object(serde_json::Map::new());
    if let Ok(args_list) = serde_json::from_str::<Vec<String>>(text) {
        for arg in args_list {
            if let Some(rest) = arg.strip_prefix("--") {
                let parts: Vec<&str> = rest.splitn(2, '=').collect();
                if parts.len() == 2 {
                    root = insert_nested(root, parts[0], parts[1]);
                } else if parts.len() == 1 {
                    root = insert_nested(root, parts[0], "true");
                }
            }
        }
    }
    root
}
fn default_parsed_item(tcx: &CfgCtxt, key: RawItemKey) -> Result<Arc<ValueMap>, ConfigError> {
    let raw_text = tcx.raw_item(key.clone())?;
    
    // Check if it's empty to avoid parsing errors
    if raw_text.trim().is_empty() {
        return Ok(Arc::new(serde_json::Value::Object(serde_json::Map::new())));
    }
    
        let parsed_value = match key.format() {
        Some(Format::Yaml) => serde_yaml::from_str(&raw_text).map_err(ConfigError::Yaml)?,
        Some(Format::Toml) => toml::from_str(&raw_text).map_err(ConfigError::Toml)?,
        Some(Format::Json) => serde_json::from_str(&raw_text).map_err(ConfigError::Json)?,
        Some(Format::Ini) => parse_ini(&raw_text)?,
        Some(Format::Properties) => parse_properties(&raw_text)?,
        None => match key {
            RawItemKey::Env(ref prefix) => parse_env(&raw_text, prefix),
            RawItemKey::Args => parse_args(&raw_text),
            _ => serde_json::Value::Null,
        },
    };
    
    // Convert root scalars to properly keyed map based on something? No, a file shouldn't be a root scalar usually, 
    // but Ini parsed as `8080` if it's not well formed maybe? 
    // Ah, `app.ini` contains "port=8080". `parse_ini` makes it `{"port": 8080}`.
    // Why did `println!("Ini Map")` output `Number(8080)`?
    // Let's check `parse_ini`.
    
    Ok(Arc::new(parsed_value))
}

fn default_merged_source(tcx: &CfgCtxt, key: SourceKey) -> Result<Arc<ValueMap>, ConfigError> {
    let mut merged = ValueMap::Object(serde_json::Map::new());
    
    // Get the registered raw items for this source
    let registry = tcx.source_registry.read().unwrap();
    if let Some(raw_items) = registry.get(&key) {
        for raw_key in raw_items {
            if let Ok(parsed) = tcx.parsed_item(raw_key.clone()) {
                if parsed.is_object() {
                    deep_merge_values(&mut merged, &parsed);
                } else if parsed.is_null() {
                    // Do nothing
                } else {
                    // It shouldn't be a scalar for a whole source, but if it is, maybe we can wrap it.
                    // Wait, args/env map parsed as scalars when path parsing goes wrong?
                    // Ah, `insert_nested` has a bug where if `parts.len() == 1`, it returns a scalar instead of wrapping it in an object!
                }
            }
        }
    }
    
    Ok(Arc::new(merged))
}

fn default_resolved_source(tcx: &CfgCtxt, key: SourceKey) -> Result<Arc<ValueMap>, ConfigError> {
    let merged = tcx.merged_source(key)?;
    
    // Perform placeholder interpolation
    let mut resolved = (*merged).clone();
    
    lazy_static! {
        // Match ${some.path.to.key}
        static ref RE: Regex = Regex::new(r"\$\{([^}]+)\}").unwrap();
    }
    
    fn resolve_value(val: &mut ValueMap, root: &ValueMap) {
        match val {
            ValueMap::String(s) => {
                if let Some(caps) = RE.captures(s) {
                    let path = caps.get(1).unwrap().as_str();
                    
                    // If the entire string is just one placeholder, we can replace it with the exact value (number, bool, etc.)
                    if s.trim() == format!("${{{}}}", path) {
                        if let Some(replacement) = get_by_path(root, path) {
                            *val = replacement.clone();
                            return;
                        }
                    }
                    
                    // Otherwise, string replacement
                    let mut new_s = s.clone();
                    for cap in RE.captures_iter(s) {
                        let p = cap.get(1).unwrap().as_str();
                        if let Some(replacement) = get_by_path(root, p) {
                            let replacement_str = match replacement {
                                ValueMap::String(rs) => rs.clone(),
                                other => other.to_string(),
                            };
                            new_s = new_s.replace(&format!("${{{}}}", p), &replacement_str);
                        }
                    }
                    *val = ValueMap::String(new_s);
                }
            }
            ValueMap::Array(arr) => {
                for item in arr {
                    resolve_value(item, root);
                }
            }
            ValueMap::Object(obj) => {
                for (_, v) in obj {
                    resolve_value(v, root);
                }
            }
            _ => {}
        }
    }
    
    // We need a clone of the root to look up against while mutating
    let root_clone = resolved.clone();
    resolve_value(&mut resolved, &root_clone);
    
    Ok(Arc::new(resolved))
}

fn get_by_path<'a>(root: &'a ValueMap, path: &str) -> Option<&'a ValueMap> {
    let mut current = root;
    for segment in path.split('.') {
        if let Some(next) = current.get(segment) {
            current = next;
        } else {
            return None;
        }
    }
    Some(current)
}

fn default_subtree(tcx: &CfgCtxt, key: SubtreeKey) -> Result<Arc<ValueMap>, ConfigError> {
    let resolved = tcx.resolved_source(key.source.clone())?;
    
    if let Some(path) = &key.path {
        if let Some(sub) = get_by_path(&resolved, path) {
            Ok(Arc::new(sub.clone()))
        } else {
            Ok(Arc::new(serde_json::Value::Null))
        }
    } else {
        Ok(resolved)
    }
}

fn default_typed_config(tcx: &CfgCtxt, key: TypedNodeKey) -> Result<Arc<dyn Any + Send + Sync>, ConfigError> {
    let value_map = tcx.subtree(key.subtree.clone())?;
    (tcx.providers.deserializer)(&value_map, &key)
}
