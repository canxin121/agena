use std::{
    fs,
    path::{Path, PathBuf},
};

use agena_storage::MemoryDir;

use super::{MemoryIndexError, MemorySearchDocument};

const INDEX_FILE: &str = "documents.json";

#[derive(Clone)]
/// Portable full-text index used on big-endian targets.
pub struct MemoryIndex {
    dir: PathBuf,
}

impl MemoryIndex {
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self {
            dir: MemoryDir::from_workspace(workspace_root).index_dir(),
        }
    }

    pub fn replace_documents(
        &self,
        documents: &[MemorySearchDocument],
    ) -> Result<(), MemoryIndexError> {
        if self.dir.exists() {
            fs::remove_dir_all(&self.dir)?;
        }
        fs::create_dir_all(&self.dir)?;
        let bytes = serde_json::to_vec(documents)?;
        fs::write(self.dir.join(INDEX_FILE), bytes)?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchDocument>, MemoryIndexError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let path = self.dir.join(INDEX_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let documents: Vec<MemorySearchDocument> = serde_json::from_slice(&fs::read(path)?)?;
        let terms = query_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut ranked = documents
            .into_iter()
            .filter_map(|document| {
                let score = document_score(&document, &terms);
                (score > 0).then_some((score, document))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(ranked
            .into_iter()
            .take(limit)
            .map(|(_, document)| document)
            .collect())
    }
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn document_score(document: &MemorySearchDocument, terms: &[String]) -> u32 {
    let name = document.name.to_lowercase();
    let description = document.description.to_lowercase();
    let searchable = document.searchable_text.to_lowercase();
    let memory_type = document.memory_type.as_deref().unwrap_or("").to_lowercase();
    terms
        .iter()
        .map(|term| {
            u32::from(name.contains(term)) * 8
                + u32::from(description.contains(term)) * 4
                + u32::from(memory_type.contains(term)) * 3
                + u32::from(searchable.contains(term)) * 2
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{document_score, query_terms};
    use crate::MemorySearchDocument;

    #[test]
    fn portable_scoring_prefers_name_matches() {
        let document = MemorySearchDocument::new(
            "1".into(),
            "Release checklist".into(),
            "Deployment notes".into(),
            None,
            "Build every target".into(),
            "memory.md".into(),
        );
        assert!(document_score(&document, &query_terms("release")) > 0);
        assert_eq!(document_score(&document, &query_terms("absent")), 0);
    }
}
