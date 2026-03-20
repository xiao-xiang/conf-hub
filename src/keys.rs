use std::any::TypeId;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Format {
    Yaml,
    Toml,
    Json,
    Ini,
    Properties,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum RawItemKey {
    File(String, Format),
    Nacos {
        group: String,
        data_id: String,
        format: Format,
    },
    Env(String), // Prefix
    Args,
}

impl RawItemKey {
    pub fn format(&self) -> Option<Format> {
        match self {
            RawItemKey::File(_, fmt) => Some(*fmt),
            RawItemKey::Nacos { format, .. } => Some(*format),
            RawItemKey::Env(_) => None,
            RawItemKey::Args => None,
        }
    }
}

pub type SourceKey = String;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SubtreeKey {
    pub source: SourceKey,
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
