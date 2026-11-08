//! Persistent full-text index for memory retrieval.
//!
//! Little-endian targets use Tantivy. Big-endian targets use a deterministic
//! JSON-backed scanner because Apache DataSketches, a Tantivy dependency, does
//! not support big-endian machines. Both backends preserve the same public API.

use serde::{Deserialize, Serialize};

mod retrieval;

/// Default passage size returned with search results, measured in Unicode characters.
pub const DEFAULT_EXCERPT_CHARS: usize = 800;

/// Upper bound on one search response. Callers can request a smaller result set.
pub const MAX_SEARCH_RESULTS: usize = 100;

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

    /// Select the most relevant body passage without splitting UTF-8 characters.
    ///
    /// An empty query selects the beginning of the body. The returned text is an
    /// exact body substring, with one-based body line numbers for navigation.
    pub fn excerpt(&self, query: &str, max_chars: usize) -> MemoryExcerpt {
        retrieval::excerpt(&self.body, &retrieval::LiteralQuery::new(query), max_chars)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A bounded, verbatim passage from a memory body.
pub struct MemoryExcerpt {
    pub text: String,
    /// One-based first line within document.body, or zero for an empty passage.
    pub start_line: usize,
    /// One-based last line within document.body, or zero for an empty passage.
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A ranked result with the evidence that can be shown to the model or user.
pub struct MemorySearchHit {
    pub document: MemorySearchDocument,
    /// Backend-local ranking score; higher is better, not a confidence probability.
    pub score: f32,
    pub excerpt: MemoryExcerpt,
    pub matched_terms: Vec<String>,
}

fn validate_documents(documents: &[MemorySearchDocument]) -> Result<(), MemoryIndexError> {
    let mut ids = std::collections::BTreeSet::new();
    for document in documents {
        if document.id.trim().is_empty() {
            return Err(MemoryIndexError::InvalidDocuments(
                "empty document id".to_owned(),
            ));
        }
        if !ids.insert(&document.id) {
            return Err(MemoryIndexError::InvalidDocuments(format!(
                "duplicate document id: {}",
                document.id
            )));
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
/// Error from the memory index.
pub enum MemoryIndexError {
    #[error("invalid memory documents: {0}")]
    InvalidDocuments(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[cfg(target_endian = "little")]
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
}

#[cfg(any(test, target_endian = "big"))]
mod portable_backend;
#[cfg(target_endian = "little")]
mod tantivy_backend;

#[cfg(target_endian = "big")]
pub use portable_backend::MemoryIndex;
#[cfg(target_endian = "little")]
pub use tantivy_backend::MemoryIndex;

#[cfg(test)]
mod tests;
