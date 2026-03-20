use std::any::TypeId;
use std::hash::{Hash, Hasher};

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

#[derive(Debug, Clone, Eq)]
pub struct TypedNodeKey {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub subtree: SubtreeKey,
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
