use std::{
    fs,
    path::{Path, PathBuf},
};

use agena_storage::MemoryDir;

use super::{
    MAX_SEARCH_RESULTS, MemoryIndexError, MemorySearchDocument, MemorySearchHit, validate_documents,
};
use crate::retrieval::{LiteralQuery, sort_hits};

const INDEX_FILE: &str = "documents.json";

#[derive(Clone)]
/// Portable full-text index used on big-endian targets.
pub struct MemoryIndex {
    dir: PathBuf,
}

impl MemoryIndex {
    // On little-endian hosts this backend is compiled only for its shared tests.
    #[cfg_attr(all(test, target_endian = "little"), allow(dead_code))]
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self {
            dir: MemoryDir::from_workspace(workspace_root).index_dir(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn replace_documents(
        &self,
        documents: &[MemorySearchDocument],
    ) -> Result<(), MemoryIndexError> {
        validate_documents(documents)?;
        agena_storage_layout::secure_directory(&self.dir)?;
        // Stable ID ordering makes an unchanged set a no-op even if the caller
        // lists its files in a different order.
        let mut documents = documents.iter().collect::<Vec<_>>();
        documents.sort_by(|left, right| left.id.cmp(&right.id));
        let bytes = serde_json::to_vec(&documents)?;
        let target = self.dir.join(INDEX_FILE);
        if fs::read(&target).is_ok_and(|current| current == bytes) {
            return Ok(());
        }
        use std::io::Write as _;
        let mut staged = tempfile::NamedTempFile::new_in(&self.dir)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged.persist(target).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchDocument>, MemoryIndexError> {
        Ok(self
            .search_with_details(query, limit)?
            .into_iter()
            .map(|hit| hit.document)
            .collect())
    }

    pub fn search_with_details(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>, MemoryIndexError> {
        let query = LiteralQuery::new(query);
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }
        let path = self.dir.join(INDEX_FILE);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let documents: Vec<MemorySearchDocument> = serde_json::from_slice(&bytes)?;
        let mut ranked = documents
            .into_iter()
            .filter_map(|mut document| {
                let score = query.evidence(&document.name).0 * 2.0
                    + query.evidence(&document.description).0
                    + query.evidence(&document.searchable_text).0
                    + query.evidence(&document.path).0;
                document
                    .searchable_ngrams
                    .clone_from(&document.searchable_text);
                (score > 0.0).then_some((score, document))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut hits = ranked
            .into_iter()
            .take(limit.min(MAX_SEARCH_RESULTS))
            .map(|(score, document)| query.hit(document, score))
            .collect::<Vec<_>>();
        sort_hits(&mut hits);
        Ok(hits)
    }
}
