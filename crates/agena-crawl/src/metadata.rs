use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use redb::{Database, ReadableDatabase, TableDefinition};

use crate::{CrawlError, StoredDocument};

const DOCUMENT_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("crawl_documents");
const URL_TO_ID_TABLE: TableDefinition<&str, &str> = TableDefinition::new("crawl_url_to_id");
const MARKDOWN_HASH_TO_ID_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("crawl_markdown_hash_to_id");
const RAW_HASH_TO_ID_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("crawl_raw_hash_to_id");

static OPEN_DATABASES: LazyLock<Mutex<HashMap<PathBuf, Arc<Database>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct CrawlMetadataStore {
    db: Arc<Database>,
}

impl CrawlMetadataStore {
    pub fn open(path: &Path) -> Result<Self, CrawlError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut open = OPEN_DATABASES.lock().map_err(|_| {
            CrawlError::InvalidInput("metadata database mutex poisoned".to_string())
        })?;
        let db = if let Some(existing) = open.get(path) {
            Arc::clone(existing)
        } else {
            let created = Arc::new(Database::create(path)?);
            open.insert(path.to_path_buf(), Arc::clone(&created));
            created
        };
        Ok(Self { db })
    }

    pub fn save_document(&self, document: &StoredDocument) -> Result<(), CrawlError> {
        let write_txn = self.db.begin_write()?;
        {
            // Older Agena versions stored the full document in redb as well
            // as JSON. Keep removing that duplicate entry as documents are
            // refreshed so metadata stays index-only going forward.
            let mut docs = write_txn.open_table(DOCUMENT_TABLE)?;
            docs.remove(document.id.as_str())?;
        }
        {
            let mut url_to_id = write_txn.open_table(URL_TO_ID_TABLE)?;
            url_to_id.insert(document.canonical_url.as_str(), document.id.as_str())?;
        }
        {
            let mut hash_to_id = write_txn.open_table(MARKDOWN_HASH_TO_ID_TABLE)?;
            hash_to_id.insert(document.markdown_hash.as_str(), document.id.as_str())?;
        }
        {
            let mut raw_hash_to_id = write_txn.open_table(RAW_HASH_TO_ID_TABLE)?;
            raw_hash_to_id.insert(document.raw_html_hash.as_str(), document.id.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn delete_document(&self, document: &StoredDocument) -> Result<(), CrawlError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut docs = write_txn.open_table(DOCUMENT_TABLE)?;
            docs.remove(document.id.as_str())?;
        }
        {
            let mut url_to_id = write_txn.open_table(URL_TO_ID_TABLE)?;
            url_to_id.remove(document.canonical_url.as_str())?;
        }
        {
            let mut hash_to_id = write_txn.open_table(MARKDOWN_HASH_TO_ID_TABLE)?;
            hash_to_id.remove(document.markdown_hash.as_str())?;
        }
        {
            let mut raw_hash_to_id = write_txn.open_table(RAW_HASH_TO_ID_TABLE)?;
            raw_hash_to_id.remove(document.raw_html_hash.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn find_document_id_by_url(
        &self,
        canonical_url: &str,
    ) -> Result<Option<String>, CrawlError> {
        let read_txn = self.db.begin_read()?;
        let url_to_id = read_txn.open_table(URL_TO_ID_TABLE)?;
        Ok(url_to_id
            .get(canonical_url)?
            .map(|value| value.value().to_string()))
    }

    pub fn find_document_id_by_markdown_hash(
        &self,
        markdown_hash: &str,
    ) -> Result<Option<String>, CrawlError> {
        let read_txn = self.db.begin_read()?;
        let hash_to_id = read_txn.open_table(MARKDOWN_HASH_TO_ID_TABLE)?;
        Ok(hash_to_id
            .get(markdown_hash)?
            .map(|value| value.value().to_string()))
    }

    pub fn find_document_id_by_raw_hash(
        &self,
        raw_hash: &str,
    ) -> Result<Option<String>, CrawlError> {
        let read_txn = self.db.begin_read()?;
        let hash_to_id = read_txn.open_table(RAW_HASH_TO_ID_TABLE)?;
        Ok(hash_to_id
            .get(raw_hash)?
            .map(|value| value.value().to_string()))
    }
}
