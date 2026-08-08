use std::fs;
use std::path::Path;

use crate::{
    CrawlDir, CrawlDocumentSummary, CrawlError, StoredDocument, metadata::CrawlMetadataStore,
    prepare_fetch_url, rebuild_search_index, search_documents,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Retention policy of the crawl store.
pub struct CrawlStoreRetention {
    pub max_documents: usize,
    pub max_total_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Report of a crawl store prune.
pub struct CrawlStorePruneReport {
    pub removed_document_count: usize,
    pub removed_bytes: u64,
    pub remaining_document_count: usize,
    pub remaining_bytes: u64,
}

#[derive(Clone)]
/// On-disk store for crawled documents.
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
        documents.sort_by_key(|document| std::cmp::Reverse(document.fetched_at));
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
        entries.sort_by_key(|(document, _)| std::cmp::Reverse(document.fetched_at));
        Ok(entries)
    }
}
