use std::fs;
use std::path::Path;

use crate::{
    CrawlDir, CrawlDocumentSummary, CrawlError, StoredDocument, metadata::CrawlMetadataStore,
    prepare_fetch_url, rebuild_search_index, search_documents,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrawlStoreRetention {
    pub max_documents: usize,
    pub max_total_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrawlStorePruneReport {
    pub removed_document_count: usize,
    pub removed_bytes: u64,
    pub remaining_document_count: usize,
    pub remaining_bytes: u64,
}

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
        Ok(())
    }

    pub fn prune(
        &self,
        retention: CrawlStoreRetention,
    ) -> Result<CrawlStorePruneReport, CrawlError> {
        self.ensure_exists()?;
        let entries = self.document_entries_with_sizes()?;
        let mut remaining_count = entries.len();
        let mut remaining_bytes = entries.iter().map(|(_, bytes)| *bytes).sum::<u64>();
        let mut removed_document_count = 0usize;
        let mut removed_bytes = 0u64;

        for (document, bytes) in entries.iter().rev() {
            if remaining_count <= retention.max_documents
                && remaining_bytes <= retention.max_total_bytes
            {
                break;
            }
            self.delete_loaded_document(document)?;
            remaining_count = remaining_count.saturating_sub(1);
            remaining_bytes = remaining_bytes.saturating_sub(*bytes);
            removed_document_count += 1;
            removed_bytes = removed_bytes.saturating_add(*bytes);
        }

        Ok(CrawlStorePruneReport {
            removed_document_count,
            removed_bytes,
            remaining_document_count: remaining_count,
            remaining_bytes,
        })
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
        Ok(self
            .list_documents()?
            .into_iter()
            .find(|document| document.markdown_hash == markdown_hash))
    }

    pub fn find_by_raw_hash(&self, raw_hash: &str) -> Result<Option<StoredDocument>, CrawlError> {
        if let Some(id) = self.metadata()?.find_document_id_by_raw_hash(raw_hash)? {
            return self.get_document(id.as_str()).map(Some);
        }
        Ok(self
            .list_documents()?
            .into_iter()
            .find(|document| document.raw_html_hash == raw_hash))
    }

    pub fn rebuild_index(&self) -> Result<(), CrawlError> {
        self.ensure_exists()?;
        let documents = self.list_documents()?;
        rebuild_search_index(self.dir.index_dir().as_path(), &documents)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::CrawlSearchHit>, CrawlError> {
        search_documents(self.dir.index_dir().as_path(), query, limit)
    }

    fn metadata(&self) -> Result<CrawlMetadataStore, CrawlError> {
        CrawlMetadataStore::open(self.dir.metadata_db_path().as_path())
    }

    fn document_path(&self, id: &str) -> std::path::PathBuf {
        self.dir.docs_dir().join(format!("{id}.json"))
    }

    fn delete_loaded_document(&self, document: &StoredDocument) -> Result<(), CrawlError> {
        let path = self.document_path(document.id.as_str());
        if path.exists() {
            fs::remove_file(path)?;
        }
        self.metadata()?.delete_document(document)?;
        Ok(())
    }

    fn document_entries_with_sizes(&self) -> Result<Vec<(StoredDocument, u64)>, CrawlError> {
        let docs_dir = self.dir.docs_dir();
        if !docs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(docs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let document = serde_json::from_slice::<StoredDocument>(&bytes)?;
            entries.push((document, bytes.len() as u64));
        }
        entries.sort_by(|(left, _), (right, _)| right.fetched_at.cmp(&left.fetched_at));
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use crate::{CrawlStore, CrawlStoreRetention, StoredDocument};

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
            hash: "hash-1".to_string(),
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
    }

    #[test]
    fn empty_metadata_database_does_not_break_url_lookup() {
        let temp = tempdir().expect("tempdir");
        let store = CrawlStore::for_workspace(temp.path());

        assert!(
            store
                .find_by_url("https://example.com/docs")
                .expect("empty metadata lookup should not fail")
                .is_none()
        );
    }

    #[test]
    fn hash_lookup_falls_back_to_document_files_when_metadata_is_empty() {
        let temp = tempdir().expect("tempdir");
        let store = CrawlStore::for_workspace(temp.path());
        let document = test_document("doc", "https://example.com/doc", 1);
        store.ensure_exists().expect("store exists");
        std::fs::write(
            store.document_path(document.id.as_str()),
            serde_json::to_vec(&document).expect("document serializes"),
        )
        .expect("document file written");

        assert_eq!(
            store
                .find_by_markdown_hash(document.markdown_hash.as_str())
                .expect("markdown lookup")
                .expect("markdown document")
                .id,
            document.id
        );
        assert_eq!(
            store
                .find_by_raw_hash(document.raw_html_hash.as_str())
                .expect("raw lookup")
                .expect("raw document")
                .id,
            document.id
        );
    }

    #[test]
    fn prune_removes_oldest_documents_and_metadata() {
        let temp = tempdir().expect("tempdir");
        let store = CrawlStore::for_workspace(temp.path());
        let old = test_document("old", "https://example.com/old", 1);
        let new = test_document("new", "https://example.com/new", 2);
        store.save_document(&old).expect("old document saved");
        store.save_document(&new).expect("new document saved");

        let report = store
            .prune(CrawlStoreRetention {
                max_documents: 1,
                max_total_bytes: u64::MAX,
            })
            .expect("prune succeeds");

        assert_eq!(report.removed_document_count, 1);
        assert!(
            store
                .find_by_url("https://example.com/old")
                .expect("find old")
                .is_none()
        );
        assert!(
            store
                .find_by_url("https://example.com/new")
                .expect("find new")
                .is_some()
        );
    }

    fn test_document(id: &str, url: &str, simhash: u64) -> StoredDocument {
        StoredDocument {
            id: id.to_string(),
            url: url.to_string(),
            canonical_url: url.to_string(),
            title: id.to_string(),
            markdown: format!("{id} content"),
            chunks: vec![format!("{id} content")],
            links: Vec::new(),
            content_type: "text/html".to_string(),
            status: 200,
            truncated: false,
            rendered: false,
            raw_html_hash: format!("raw-{id}"),
            markdown_hash: format!("markdown-{id}"),
            simhash,
            etag: None,
            last_modified: None,
            chunk_hashes: vec![format!("chunk-{id}")],
            hash: format!("markdown-{id}"),
            depth: 0,
            fetched_at: Utc::now() + chrono::Duration::seconds(simhash as i64),
        }
    }
}
