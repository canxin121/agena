//! Rebuildable full-text cache over the caller's authorized memory records.

use agena_domain::MemoryKind;
use serde::{Deserialize, Serialize};

mod retrieval;
mod tantivy_backend;

pub use tantivy_backend::MemoryIndex;

/// Default passage size returned with search results, measured in Unicode characters.
pub const DEFAULT_EXCERPT_CHARS: usize = 800;

/// Upper bound on one search response. Callers can request a smaller result set.
pub const MAX_SEARCH_RESULTS: usize = 100;

/// A canonical record projected from the authorized memory store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemorySearchDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: MemoryKind,
    pub body: String,
    pub path: String,
}

impl MemorySearchDocument {
    pub fn new(
        id: String,
        name: String,
        description: String,
        kind: MemoryKind,
        body: String,
        path: String,
    ) -> Self {
        Self {
            id,
            name,
            description,
            kind,
            body,
            path,
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

/// A bounded, verbatim passage from a memory body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryExcerpt {
    pub text: String,
    /// One-based first line within document.body, or zero for an empty passage.
    pub start_line: usize,
    /// One-based last line within document.body, or zero for an empty passage.
    pub end_line: usize,
}

/// A ranked result with the evidence that can be shown to the model or user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySearchHit {
    pub document: MemorySearchDocument,
    /// Ranking score; higher is better, not a confidence probability.
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

/// Error from the memory index.
#[derive(Debug, thiserror::Error)]
pub enum MemoryIndexError {
    #[error("invalid memory documents: {0}")]
    InvalidDocuments(String),
    #[error("invalid memory cache: {0}")]
    InvalidCache(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
}

#[cfg(test)]
mod tests;
