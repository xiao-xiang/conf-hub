use crate::context::CfgCtxt;
use crate::error::ConfigError;
use crate::keys::{SubtreeKey, TypedNodeKey};
use crate::parsers::deep_merge_values;
use serde_json::Value as ValueMap;
use std::any::Any;
use std::sync::Arc;
use regex::Regex;
use lazy_static::lazy_static;

pub struct CfgProviders {
    pub source_ast: fn(&CfgCtxt, String) -> Result<Arc<ValueMap>, ConfigError>,
    pub merged_global: fn(&CfgCtxt) -> Result<Arc<ValueMap>, ConfigError>,
    pub resolved_global: fn(&CfgCtxt) -> Result<Arc<ValueMap>, ConfigError>,
    pub subtree: fn(&CfgCtxt, SubtreeKey) -> Result<Arc<ValueMap>, ConfigError>,
    pub typed_config: fn(&CfgCtxt, TypedNodeKey) -> Result<Arc<dyn Any + Send + Sync>, ConfigError>,
}

impl Default for CfgProviders {
    fn default() -> Self {
        Self {
            source_ast: default_source_ast,
            merged_global: default_merged_global,
            resolved_global: default_resolved_global,
            subtree: default_subtree,
            typed_config: default_typed_config,
        }
    }
}

fn default_source_ast(tcx: &CfgCtxt, node_id: String) -> Result<Arc<ValueMap>, ConfigError> {
    let provider = tcx.node_providers.get(&node_id)
        .ok_or_else(|| ConfigError::Provider(format!("Provider not found: {}", node_id)))?;
    provider.fetch_and_parse()
}

fn default_merged_global(tcx: &CfgCtxt) -> Result<Arc<ValueMap>, ConfigError> {
    let mut merged = ValueMap::Object(serde_json::Map::new());
    
    // Get the flat list of global sources
    let global_sources = &tcx.global_source_ids;
    for node_id in global_sources.iter() {
        let parsed = tcx.source_ast(node_id.clone())?;
        if parsed.is_object() {
            deep_merge_values(&mut merged, &parsed);
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
