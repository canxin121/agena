use std::fs;
use std::path::Path;

use crate::{
    CrawlDir, CrawlDocumentSummary, CrawlError, StoredDocument, metadata::CrawlMetadataStore,
    prepare_fetch_url, rebuild_search_index, search_documents, vector_index::CrawlVectorIndex,
};

#[derive(Clone)]
pub struct CrawlStore {
    dir: CrawlDir,
}

impl CrawlStore {
    pub fn for_workspace(workspace_root: &Path) -> Self {
        Self {
            dir: CrawlDir::from_workspace(workspace_root),
        }
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn ensure_exists(&self) -> Result<(), CrawlError> {
        self.dir.ensure_exists()?;
        Ok(())
    }

    pub fn save_document(&self, document: &StoredDocument) -> Result<(), CrawlError> {
        self.ensure_exists()?;
        let path = self.document_path(document.id.as_str());
        let temp_path = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(document)?;
        fs::write(&temp_path, bytes)?;
        fs::rename(temp_path, path)?;
        self.metadata()?.save_document(document)?;
        self.vector_index()?.replace_document(document)?;
        Ok(())
    }

    pub fn list_documents(&self) -> Result<Vec<StoredDocument>, CrawlError> {
        let docs_dir = self.dir.docs_dir();
        if !docs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut documents = Vec::new();
        for entry in fs::read_dir(docs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(path)?;
            let document = serde_json::from_slice::<StoredDocument>(&bytes)?;
            documents.push(document);
        }
        documents.sort_by(|left, right| right.fetched_at.cmp(&left.fetched_at));
        Ok(documents)
    }

    pub fn list_summaries(&self, limit: usize) -> Result<Vec<CrawlDocumentSummary>, CrawlError> {
        Ok(self
            .list_documents()?
            .into_iter()
            .take(limit)
            .map(|document| document.summary())
            .collect())
    }

    pub fn get_document(&self, id: &str) -> Result<StoredDocument, CrawlError> {
        if let Some(document) = self.metadata()?.get_document(id)? {
            return Ok(document);
        }
        let path = self.document_path(id);
        if !path.exists() {
            return Err(CrawlError::NotFound(format!("crawl document '{id}'")));
        }
        let bytes = fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn find_by_url(&self, raw_url: &str) -> Result<Option<StoredDocument>, CrawlError> {
        let target = prepare_fetch_url(raw_url)?.to_string();
        if let Some(id) = self.metadata()?.find_document_id_by_url(target.as_str())? {
            return self.get_document(id.as_str()).map(Some);
        }
        Ok(self
            .list_documents()?
            .into_iter()
            .find(|document| document.canonical_url == target))
    }

    pub fn find_by_markdown_hash(
        &self,
        markdown_hash: &str,
    ) -> Result<Option<StoredDocument>, CrawlError> {
        if let Some(id) = self
            .metadata()?
            .find_document_id_by_markdown_hash(markdown_hash)?
        {
            return self.get_document(id.as_str()).map(Some);
        }
        Ok(None)
    }

    pub fn find_by_raw_hash(&self, raw_hash: &str) -> Result<Option<StoredDocument>, CrawlError> {
        if let Some(id) = self.metadata()?.find_document_id_by_raw_hash(raw_hash)? {
            return self.get_document(id.as_str()).map(Some);
        }
        Ok(None)
    }

    pub fn rebuild_index(&self) -> Result<(), CrawlError> {
        self.ensure_exists()?;
        let documents = self.list_documents()?;
        rebuild_search_index(self.dir.index_dir().as_path(), &documents)?;
        self.vector_index()?.rebuild(&documents)?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::CrawlSearchHit>, CrawlError> {
        search_documents(self.dir.index_dir().as_path(), query, limit)
    }

    pub fn vector_search(
        &self,
        query_vector: &[f32],
        embedding_model: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::CrawlSearchHit>, CrawlError> {
        self.vector_index()?
            .search(query_vector, embedding_model, limit)
    }

    fn metadata(&self) -> Result<CrawlMetadataStore, CrawlError> {
        CrawlMetadataStore::open(self.dir.metadata_db_path().as_path())
    }

    fn vector_index(&self) -> Result<CrawlVectorIndex, CrawlError> {
        CrawlVectorIndex::open(self.dir.vector_db_path().as_path())
    }

    fn document_path(&self, id: &str) -> std::path::PathBuf {
        self.dir.docs_dir().join(format!("{id}.json"))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use crate::{CrawlStore, StoredDocument};

    #[test]
    fn save_find_and_search_round_trip() {
        let temp = tempdir().expect("tempdir");
        let store = CrawlStore::for_workspace(temp.path());
        let document = StoredDocument {
            id: "doc-1".to_string(),
            url: "https://example.com/docs".to_string(),
            canonical_url: "https://example.com/docs".to_string(),
            title: "Agena Crawl Docs".to_string(),
            markdown: "Agena crawl stores markdown locally and builds a Tantivy index.".to_string(),
            chunks: vec![
                "Agena crawl stores markdown locally and builds a Tantivy index.".to_string(),
            ],
            links: vec!["https://example.com/docs/install".to_string()],
            content_type: "text/html".to_string(),
            status: 200,
            truncated: false,
            rendered: false,
            raw_html_hash: "raw-hash-1".to_string(),
            markdown_hash: "hash-1".to_string(),
            simhash: 1,
            etag: None,
            last_modified: None,
            chunk_hashes: vec!["chunk-hash-1".to_string()],
            chunk_embeddings: vec![vec![1.0, 0.0]],
            hash: "hash-1".to_string(),
            embedding_model: Some("test-model".to_string()),
            embedding_dimension: Some(2),
            depth: 0,
            fetched_at: Utc::now(),
        };

        store.save_document(&document).expect("document saved");
        store.rebuild_index().expect("index rebuilt");

        let found = store
            .find_by_url("https://example.com/docs?utm_source=test")
            .expect("find succeeds")
            .expect("document exists");
        assert_eq!(found.id, document.id);

        let hits = store.search("Tantivy index", 5).expect("search succeeds");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, document.id);

        let vector_hits = store
            .vector_search(&[1.0, 0.0], Some("test-model"), 5)
            .expect("vector search succeeds");
        assert_eq!(vector_hits.len(), 1);
        assert_eq!(vector_hits[0].id, document.id);
    }
}
