use std::any::{Any, TypeId};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use crate::error::ConfigError;
use serde_json::Value as ValueMap;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RawItemKey {
    pub uri: String,
    pub parser_type: String,
}

impl RawItemKey {
    pub fn new(uri: impl Into<String>, parser_type: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            parser_type: parser_type.into(),
        }
    }
}

// SubtreeKey no longer needs a SourceKey because it always refers to the global tree
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SubtreeKey {
    pub path: Option<String>,
}

#[derive(Clone)]
pub struct TypedNodeKey {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub subtree: SubtreeKey,
    // The type-erased factory method
    pub deserializer: fn(&ValueMap) -> Result<Arc<dyn Any + Send + Sync>, ConfigError>,
}

impl std::fmt::Debug for TypedNodeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedNodeKey")
            .field("type_id", &self.type_id)
            .field("type_name", &self.type_name)
            .field("subtree", &self.subtree)
            .finish()
    }
}

impl Hash for TypedNodeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
        self.subtree.hash(state);
    }
}

impl PartialEq for TypedNodeKey {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id && self.subtree == other.subtree
    }
}

impl Eq for TypedNodeKey {}
