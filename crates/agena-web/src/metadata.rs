use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use redb::{
    Database, MultimapTableHandle, ReadableDatabase, ReadableTable, TableDefinition, TableError,
    TableHandle,
};

use crate::{CrawlError, StoredDocument};

const URL_TO_ID_TABLE: TableDefinition<&str, &str> = TableDefinition::new("web_url_to_id");
const MARKDOWN_HASH_TO_ID_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("web_markdown_hash_to_id");
const RAW_HASH_TO_ID_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("web_raw_hash_to_id");

static OPEN_DATABASES: LazyLock<Mutex<HashMap<PathBuf, Weak<DatabaseSlot>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct DatabaseSlot {
    database: Mutex<Weak<Database>>,
}

/// Store of crawl metadata.
#[derive(Clone)]
pub struct CrawlMetadataStore {
    db: Option<Arc<Database>>,
    _slot: Arc<DatabaseSlot>,
}

impl Drop for CrawlMetadataStore {
    fn drop(&mut self) {
        // redb's last-handle close flushes synchronously. Async owners hand
        // this entire handle to a cleanup worker before dropping it. Keep
        // opening this file serialized until the actual close has finished.
        let _guard = self
            ._slot
            .database
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(self.db.take());
    }
}

impl CrawlMetadataStore {
    fn db(&self) -> &Database {
        self.db.as_deref().expect("metadata database is open")
    }
    pub fn open(path: &Path) -> Result<Self, CrawlError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let parent = path
            .parent()
            .ok_or_else(|| CrawlError::InvalidInput("metadata path has no parent".into()))?;
        let name = path
            .file_name()
            .ok_or_else(|| CrawlError::InvalidInput("metadata path has no filename".into()))?;
        let path = parent.canonicalize()?.join(name);
        let slot = {
            let mut open = OPEN_DATABASES.lock().map_err(|_| {
                CrawlError::InvalidInput("metadata database registry mutex poisoned".into())
            })?;
            open.retain(|_, slot| slot.strong_count() > 0);
            if let Some(slot) = open.get(&path).and_then(Weak::upgrade) {
                slot
            } else {
                let slot = Arc::new(DatabaseSlot::default());
                open.insert(path.clone(), Arc::downgrade(&slot));
                slot
            }
        };
        // File opening and schema I/O must not hold the registry lock for
        // every workspace. Only callers opening this same file serialize.
        let mut retained = slot.database.lock().map_err(|_| {
            CrawlError::InvalidInput("metadata database initialization mutex poisoned".into())
        })?;
        let db = if let Some(existing) = retained.upgrade() {
            existing
        } else {
            let created = Arc::new(Database::create(&path)?);
            *retained = Arc::downgrade(&created);
            created
        };
        drop(retained);
        let store = Self {
            db: Some(db),
            _slot: slot,
        };
        store.initialize_or_validate_tables()?;
        Ok(store)
    }

    fn initialize_or_validate_tables(&self) -> Result<(), CrawlError> {
        const EXPECTED_TABLES: [&str; 3] = [
            "web_url_to_id",
            "web_markdown_hash_to_id",
            "web_raw_hash_to_id",
        ];

        let read_txn = self.db().begin_read()?;
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

        let write_txn = self.db().begin_write()?;
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

    pub fn save_document(
        &self,
        document: &StoredDocument,
        previous: Option<&StoredDocument>,
    ) -> Result<(), CrawlError> {
        let write_txn = self.db().begin_write()?;
        if let Some(previous) = previous {
            remove_owned_mappings(&write_txn, previous)?;
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
        let write_txn = self.db().begin_write()?;
        remove_owned_mappings(&write_txn, document)?;
        write_txn.commit()?;
        Ok(())
    }

    pub fn find_document_id_by_url(
        &self,
        canonical_url: &str,
    ) -> Result<Option<String>, CrawlError> {
        let read_txn = self.db().begin_read()?;
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
        let read_txn = self.db().begin_read()?;
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
        let read_txn = self.db().begin_read()?;
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

fn remove_owned_mappings(
    transaction: &redb::WriteTransaction,
    document: &StoredDocument,
) -> Result<(), CrawlError> {
    for (definition, key) in [
        (URL_TO_ID_TABLE, document.canonical_url.as_str()),
        (MARKDOWN_HASH_TO_ID_TABLE, document.markdown_hash.as_str()),
        (RAW_HASH_TO_ID_TABLE, document.raw_html_hash.as_str()),
    ] {
        let mut table = transaction.open_table(definition)?;
        let owned = table
            .get(key)?
            .is_some_and(|value| value.value() == document.id);
        if owned {
            table.remove(key)?;
        }
    }
    Ok(())
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
            table_names(store.db()),
            vec![
                "web_markdown_hash_to_id".to_owned(),
                "web_raw_hash_to_id".to_owned(),
                "web_url_to_id".to_owned(),
            ]
        );
    }

    #[test]
    fn metadata_handles_share_aliases_and_release_the_database_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.redb");
        let first = CrawlMetadataStore::open(&path).unwrap();
        let second = CrawlMetadataStore::open(&dir.path().join(".").join("metadata.redb")).unwrap();
        assert!(Arc::ptr_eq(
            first.db.as_ref().unwrap(),
            second.db.as_ref().unwrap()
        ));
        let weak = Arc::downgrade(first.db.as_ref().unwrap());
        drop(first);
        assert!(weak.upgrade().is_some());
        drop(second);
        assert!(weak.upgrade().is_none());
        Database::create(&path).expect("last owner must release the file lock");
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
