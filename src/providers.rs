use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{RawItemKey, SubtreeKey, TypedNodeKey};
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
    pub merged_global: fn(&CfgCtxt) -> Result<Arc<ValueMap>, ConfigError>,
    pub resolved_global: fn(&CfgCtxt) -> Result<Arc<ValueMap>, ConfigError>,
    pub subtree: fn(&CfgCtxt, SubtreeKey) -> Result<Arc<ValueMap>, ConfigError>,
    pub typed_config: fn(&CfgCtxt, TypedNodeKey) -> Result<Arc<dyn Any + Send + Sync>, ConfigError>,
}

impl Default for CfgProviders {
    fn default() -> Self {
        Self {
            raw_item: default_raw_item,
            parsed_item: default_parsed_item,
            merged_global: default_merged_global,
            resolved_global: default_resolved_global,
            subtree: default_subtree,
            typed_config: default_typed_config,
        }
    }
}

fn default_raw_item(tcx: &CfgCtxt, key: RawItemKey) -> Result<Arc<String>, ConfigError> {
    // If it's a file, read from disk
    if key.uri.starts_with("file://") {
        let path = key.uri.trim_start_matches("file://");
        let content = fs::read_to_string(path).map_err(ConfigError::Io)?;
        return Ok(Arc::new(content));
    }
    
    // Otherwise, check raw store (dynamic sources like Nacos, Env, Args)
    let store = tcx.raw_store.read().unwrap();
    if let Some(content) = store.get(&key) {
        Ok(Arc::new(content.clone()))
    } else {
        Ok(Arc::new(String::new()))
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

fn parse_env(text: &str) -> ValueMap {
    let mut root = ValueMap::Object(serde_json::Map::new());
    if let Ok(env_map) = serde_json::from_str::<HashMap<String, String>>(text) {
        for (k, v) in env_map {
            // For env_kv, we don't know the prefix here unless passed,
            // but in the new design the prefix filtering should be done by the connector!
            // The EnvConnector should only pass the filtered and prefix-stripped map here.
            let path = k.to_lowercase().replace("__", ".");
            root = insert_nested(root, &path, &v);
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
    
    let parsed_value = match key.parser_type.as_str() {
        "yaml" => serde_yaml::from_str(&raw_text).map_err(ConfigError::Yaml)?,
        "toml" => toml::from_str(&raw_text).map_err(ConfigError::Toml)?,
        "json" => serde_json::from_str(&raw_text).map_err(ConfigError::Json)?,
        "ini" => parse_ini(&raw_text)?,
        "properties" => parse_properties(&raw_text)?,
        "env_kv" => parse_env(&raw_text),
        "args_kv" => parse_args(&raw_text),
        _ => serde_json::Value::Null,
    };
    
    Ok(Arc::new(parsed_value))
}

fn default_merged_global(tcx: &CfgCtxt) -> Result<Arc<ValueMap>, ConfigError> {
    let mut merged = ValueMap::Object(serde_json::Map::new());
    
    // Get the flat list of global sources
    let global_sources = tcx.global_sources.read().unwrap();
    for raw_key in global_sources.iter() {
        let parsed = tcx.parsed_item(raw_key.clone())?;
        if parsed.is_object() {
            deep_merge_values(&mut merged, &parsed);
        } else if parsed.is_null() {
            // Do nothing
        } else {
            // It shouldn't be a scalar for a whole source, but if it is, maybe we can wrap it.
        }
    }
    
    Ok(Arc::new(merged))
}

fn default_resolved_global(tcx: &CfgCtxt) -> Result<Arc<ValueMap>, ConfigError> {
    let merged = tcx.merged_global()?;
    
    // Perform placeholder interpolation
    let mut resolved = (*merged).clone();
    
    lazy_static! {
        // Match ${some.path.to.key}
        static ref RE: Regex = Regex::new(r"\$\{([^}]+)\}").unwrap();
    }
    
    fn resolve_value(val: &mut ValueMap, root: &ValueMap, depth: usize) {
        if depth > 10 {
            return; // Prevent infinite recursion
        }
        match val {
            ValueMap::String(s) => {
                if let Some(caps) = RE.captures(s) {
                    let path = caps.get(1).unwrap().as_str();
                    
                    // If the entire string is just one placeholder, we can replace it with the exact value (number, bool, etc.)
                    if s.trim() == format!("${{{}}}", path) {
                        if let Some(replacement) = get_by_path(root, path) {
                            let mut new_val = replacement.clone();
                            // Recursively resolve the replacement
                            resolve_value(&mut new_val, root, depth + 1);
                            *val = new_val;
                            return;
                        }
                    }
                    
                    // Otherwise, string replacement
                    let mut new_s = s.clone();
                    for cap in RE.captures_iter(s) {
                        let p = cap.get(1).unwrap().as_str();
                        if let Some(replacement) = get_by_path(root, p) {
                            // Recursively resolve the replacement string before replacing
                            let mut rep_clone = replacement.clone();
                            resolve_value(&mut rep_clone, root, depth + 1);
                            
                            let replacement_str = match rep_clone {
                                ValueMap::String(rs) => rs,
                                other => other.to_string(),
                            };
                            new_s = new_s.replace(&format!("${{{}}}", p), &replacement_str);
                        }
                    }
                    
                    // After string replacement, the new string might still contain placeholders
                    // (e.g. if a placeholder evaluated to another string with placeholders).
                    // We can recursively resolve it.
                    let mut final_val = ValueMap::String(new_s);
                    if let ValueMap::String(ref final_s) = final_val {
                        if RE.is_match(final_s) {
                            resolve_value(&mut final_val, root, depth + 1);
                        }
                    }
                    *val = final_val;
                }
            }
            ValueMap::Array(arr) => {
                for item in arr {
                    resolve_value(item, root, depth);
                }
            }
            ValueMap::Object(obj) => {
                for (_, v) in obj {
                    resolve_value(v, root, depth);
                }
            }
            _ => {}
        }
    }
    
    // We need a clone of the root to look up against while mutating
    let root_clone = resolved.clone();
    resolve_value(&mut resolved, &root_clone, 0);
    
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
    let resolved = tcx.resolved_global()?;
    
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
    (key.deserializer)(&value_map)
}
