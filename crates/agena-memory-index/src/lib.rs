//! Persistent full-text index for memory retrieval.
//!
//! Little-endian targets use Tantivy. Big-endian targets use a deterministic
//! JSON-backed scanner because Apache DataSketches, a Tantivy dependency, does
//! not support big-endian machines. Both backends preserve the same public API.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A document indexed for memory search.
pub struct MemorySearchDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    pub memory_type: Option<String>,
    pub body: String,
    pub path: String,
    pub searchable_text: String,
    #[serde(skip_serializing, skip_deserializing)]
    searchable_ngrams: String,
}

impl MemorySearchDocument {
    pub fn new(
        id: String,
        name: String,
        description: String,
        memory_type: Option<String>,
        body: String,
        path: String,
    ) -> Self {
        let searchable_text = format!(
            "{} {} {} {}",
            name,
            description,
            memory_type.as_deref().unwrap_or(""),
            body
        );
        Self {
            id,
            name,
            description,
            memory_type,
            body,
            path,
            searchable_ngrams: searchable_text.clone(),
            searchable_text,
        }
    }
}

#[derive(Debug, thiserror::Error)]
/// Error from the memory index.
pub enum MemoryIndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[cfg(target_endian = "little")]
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
}

#[cfg(target_endian = "big")]
mod portable_backend;
#[cfg(target_endian = "little")]
mod tantivy_backend;

#[cfg(target_endian = "big")]
pub use portable_backend::MemoryIndex;
#[cfg(target_endian = "little")]
pub use tantivy_backend::MemoryIndex;
