use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use redb::{
    Database, MultimapTableHandle, ReadableDatabase, TableDefinition, TableError, TableHandle,
};

use crate::{CrawlError, StoredDocument};

const URL_TO_ID_TABLE: TableDefinition<&str, &str> = TableDefinition::new("web_url_to_id");
const MARKDOWN_HASH_TO_ID_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("web_markdown_hash_to_id");
const RAW_HASH_TO_ID_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("web_raw_hash_to_id");

static OPEN_DATABASES: LazyLock<Mutex<HashMap<PathBuf, Arc<Database>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Store of crawl metadata.
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
        let store = Self { db };
        store.initialize_or_validate_tables()?;
        Ok(store)
    }

    fn initialize_or_validate_tables(&self) -> Result<(), CrawlError> {
        const EXPECTED_TABLES: [&str; 3] = [
            "web_url_to_id",
            "web_markdown_hash_to_id",
            "web_raw_hash_to_id",
        ];

        let read_txn = self.db.begin_read()?;
        let mut tables = read_txn
            .list_tables()?
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        tables.sort();
        let mut multimaps = read_txn
            .list_multimap_tables()?
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        multimaps.sort();

        if !tables.is_empty() || !multimaps.is_empty() {
            let mut expected = EXPECTED_TABLES.map(str::to_owned).to_vec();
            expected.sort();
            if tables != expected || !multimaps.is_empty() {
                return Err(CrawlError::InvalidInput(format!(
                    "crawl metadata database schema does not match this Agena build; delete the database and start with a fresh one (tables: {tables:?}, multimap_tables: {multimaps:?})"
                )));
            }
            let _ = read_txn.open_table(URL_TO_ID_TABLE)?;
            let _ = read_txn.open_table(MARKDOWN_HASH_TO_ID_TABLE)?;
            let _ = read_txn.open_table(RAW_HASH_TO_ID_TABLE)?;
            return Ok(());
        }
        drop(read_txn);

        let write_txn = self.db.begin_write()?;
        {
            let _ = write_txn.open_table(URL_TO_ID_TABLE)?;
        }
        {
            let _ = write_txn.open_table(MARKDOWN_HASH_TO_ID_TABLE)?;
        }
        {
            let _ = write_txn.open_table(RAW_HASH_TO_ID_TABLE)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn save_document(&self, document: &StoredDocument) -> Result<(), CrawlError> {
        let write_txn = self.db.begin_write()?;
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
        let url_to_id = match read_txn.open_table(URL_TO_ID_TABLE) {
            Ok(table) => table,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(url_to_id
            .get(canonical_url)?
            .map(|value| value.value().to_string()))
    }

    pub fn find_document_id_by_markdown_hash(
        &self,
        markdown_hash: &str,
    ) -> Result<Option<String>, CrawlError> {
        let read_txn = self.db.begin_read()?;
        let hash_to_id = match read_txn.open_table(MARKDOWN_HASH_TO_ID_TABLE) {
            Ok(table) => table,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(hash_to_id
            .get(markdown_hash)?
            .map(|value| value.value().to_string()))
    }

    pub fn find_document_id_by_raw_hash(
        &self,
        raw_hash: &str,
    ) -> Result<Option<String>, CrawlError> {
        let read_txn = self.db.begin_read()?;
        let hash_to_id = match read_txn.open_table(RAW_HASH_TO_ID_TABLE) {
            Ok(table) => table,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(hash_to_id
            .get(raw_hash)?
            .map(|value| value.value().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBSOLETE_TABLE: TableDefinition<&str, &str> = TableDefinition::new("obsolete_table");
    const WRONG_URL_TO_ID_TABLE: TableDefinition<u64, u64> = TableDefinition::new("web_url_to_id");

    fn table_names(db: &Database) -> Vec<String> {
        let txn = db.begin_read().expect("read metadata schema");
        let mut names = txn
            .list_tables()
            .expect("list metadata tables")
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn fresh_metadata_database_uses_only_the_current_tables() {
        let dir = tempfile::tempdir().expect("metadata tempdir");
        let path = dir.path().join("metadata.redb");
        let store = CrawlMetadataStore::open(&path).expect("create current metadata database");
        assert_eq!(
            table_names(store.db.as_ref()),
            vec![
                "web_markdown_hash_to_id".to_owned(),
                "web_raw_hash_to_id".to_owned(),
                "web_url_to_id".to_owned(),
            ]
        );
    }

    #[test]
    fn non_current_metadata_database_is_rejected_without_repair() {
        let dir = tempfile::tempdir().expect("metadata tempdir");
        let path = dir.path().join("metadata.redb");
        let db = Database::create(&path).expect("create fixture metadata database");
        let txn = db.begin_write().expect("begin fixture schema");
        {
            let _ = txn
                .open_table(URL_TO_ID_TABLE)
                .expect("create one current table");
            let _ = txn
                .open_table(OBSOLETE_TABLE)
                .expect("create obsolete table");
        }
        txn.commit().expect("commit fixture schema");
        drop(db);

        let error = match CrawlMetadataStore::open(&path) {
            Ok(_) => panic!("non-current metadata schema must be rejected"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("does not match this Agena build"),
            "{error}"
        );
    }

    #[test]
    fn metadata_table_type_mismatch_is_rejected() {
        let dir = tempfile::tempdir().expect("metadata tempdir");
        let path = dir.path().join("metadata.redb");
        let db = Database::create(&path).expect("create fixture metadata database");
        let txn = db.begin_write().expect("begin fixture schema");
        {
            let _ = txn
                .open_table(WRONG_URL_TO_ID_TABLE)
                .expect("create wrong typed url table");
            let _ = txn
                .open_table(MARKDOWN_HASH_TO_ID_TABLE)
                .expect("create markdown hash table");
            let _ = txn
                .open_table(RAW_HASH_TO_ID_TABLE)
                .expect("create raw hash table");
        }
        txn.commit().expect("commit fixture schema");
        drop(db);

        assert!(CrawlMetadataStore::open(&path).is_err());
    }
}
