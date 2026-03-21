use crate::error::ConfigError;
use serde_json::Value as ValueMap;
use ini::Ini;
use java_properties::read;
use std::collections::HashMap;

pub fn insert_nested(root: ValueMap, path: &str, val: &str) -> ValueMap {
    let parts: Vec<&str> = path.split('.').collect();
    
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
    
    if parts.is_empty() || parts[0].is_empty() {
        return root;
    }
    
    let new_tree = build_tree(&parts, val);
    let mut cloned_root = root.clone();
    
    if !cloned_root.is_object() {
        cloned_root = ValueMap::Object(serde_json::Map::new());
    }
    
    deep_merge_values(&mut cloned_root, &new_tree);
    cloned_root
}

pub fn deep_merge_values(target: &mut ValueMap, source: &ValueMap) {
    match (target, source) {
        (ValueMap::Object(a), ValueMap::Object(b)) => {
            for (k, v) in b {
                let entry = a.entry(k.clone()).or_insert(ValueMap::Null);
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

pub fn parse_ini(text: &str) -> Result<ValueMap, ConfigError> {
    let mut root = ValueMap::Object(serde_json::Map::new());
    let ini = Ini::load_from_str(text).map_err(|e| ConfigError::Provider(format!("INI parse error: {}", e)))?;
    
    for (sec, prop) in ini.iter() {
        let section_name = sec.unwrap_or("");
        for (k, v) in prop.iter() {
            let path = if section_name.is_empty() {
                k.to_string()
            } else {
                format!("{}.{}", section_name, k)
            };
            root = insert_nested(root, &path, v);
        }
    }
    Ok(root)
}

pub fn parse_properties(text: &str) -> Result<ValueMap, ConfigError> {
    let mut root = ValueMap::Object(serde_json::Map::new());
    let props = read(text.as_bytes()).map_err(|e| ConfigError::Provider(format!("Properties parse error: {}", e)))?;
    
    for (k, v) in props {
        root = insert_nested(root, &k, &v);
    }
    Ok(root)
}

pub fn parse_env(text: &str) -> ValueMap {
    let mut root = ValueMap::Object(serde_json::Map::new());
    if let Ok(env_map) = serde_json::from_str::<HashMap<String, String>>(text) {
        for (k, v) in env_map {
            let path = k.to_lowercase().replace("__", ".");
            root = insert_nested(root, &path, &v);
        }
    }
    root
}

pub fn parse_args(text: &str) -> ValueMap {
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
