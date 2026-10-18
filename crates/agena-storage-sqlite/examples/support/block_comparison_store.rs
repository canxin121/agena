//! Experimental payload placement only. SQLite remains the atomic reference
//! and receipt root for both adapters. Never included by production modules.

use std::{
    collections::HashMap,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use agena_storage::kernel::EventRecord;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
    TransactionTrait, Value,
};
use serde::Serialize;
use serde_json::{Value as Json, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::{
    AnyResult,
    fixture::{Block, Item, Spec, batch_digest},
};

pub const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ENTRIES: usize = 2048;
pub const READ_PERMITS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    Sqlite,
    Files,
}
impl Adapter {
    pub fn parse(value: &str) -> AnyResult<Self> {
        match value {
            "sqlite" => Ok(Self::Sqlite),
            "files" => Ok(Self::Files),
            _ => Err("unknown comparison adapter".into()),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Files => "files",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Prepared,
    Transaction,
    Committed,
}
impl Fault {
    pub fn parse(value: &str) -> AnyResult<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "transaction" => Ok(Self::Transaction),
            "committed" => Ok(Self::Committed),
            _ => Err("unknown comparison fault boundary".into()),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Transaction => "transaction",
            Self::Committed => "committed",
        }
    }
}

async fn pause_at(requested: Option<Fault>, current: Fault) -> AnyResult<()> {
    if requested == Some(current) {
        // The external runner SIGKILLs this child after seeing the exact
        // boundary. No Rust destructor, graceful SQLite close, or cleanup runs.
        println!(
            "{}",
            json!({"phase":"fault_ready","boundary":current.name()})
        );
        std::io::stdout().flush()?;
        std::future::pending::<()>().await;
    }
    Ok(())
}

pub async fn connect(root: &Path, connections: u32) -> AnyResult<DatabaseConnection> {
    let mut options = ConnectOptions::new(format!(
        "sqlite://{}?mode=rwc",
        root.join("blocks.db").display()
    ));
    options
        .max_connections(connections)
        .min_connections(1)
        .sqlx_logging(false)
        .map_sqlx_sqlite_opts(|opts| {
            use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
            opts.journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Full)
                .foreign_keys(true)
                .pragma("cache_size", "-8192")
                .pragma("mmap_size", "0")
                .pragma("busy_timeout", "5000")
                .pragma("wal_autocheckpoint", "1000")
        });
    let db = Database::connect(options).await?;
    let synchronous: i64 = db
        .query_one(sql("PRAGMA synchronous"))
        .await?
        .ok_or("missing synchronous pragma")?
        .try_get("", "synchronous")?;
    let journal: String = db
        .query_one(sql("PRAGMA journal_mode"))
        .await?
        .ok_or("missing journal pragma")?
        .try_get("", "journal_mode")?;
    if synchronous != 2 || journal != "wal" {
        return Err("comparison requires WAL/FULL".into());
    }
    Ok(db)
}

fn sql(value: &str) -> Statement {
    Statement::from_string(DatabaseBackend::Sqlite, value)
}
fn bound(value: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, value, values)
}

#[derive(Default, Serialize)]
pub struct Writes {
    pub sqlite_transactions: u64,
    pub reference_receipt_retries: u64,
    pub committed_blocks: u64,
    pub committed_encoded_bytes: u64,
    pub merged_files_created: u64,
    pub file_payload_write_bytes: u64,
    pub file_fsync_calls: u64,
    pub directory_fsync_calls: u64,
    pub wal_peak_file_bytes: u64,
}

#[derive(Default)]
pub struct Reads {
    metadata_queries: AtomicU64,
    payload_queries: AtomicU64,
    metadata_rows: AtomicU64,
    fetched_payload_bytes: AtomicU64,
    file_payload_read_bytes: AtomicU64,
    decoded_events: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}
impl Reads {
    pub fn summary(&self) -> Json {
        json!({"metadata_queries":self.metadata_queries.load(Ordering::Relaxed),"payload_queries":self.payload_queries.load(Ordering::Relaxed),"metadata_rows":self.metadata_rows.load(Ordering::Relaxed),"fetched_payload_bytes":self.fetched_payload_bytes.load(Ordering::Relaxed),"file_payload_read_bytes":self.file_payload_read_bytes.load(Ordering::Relaxed),"decoded_events":self.decoded_events.load(Ordering::Relaxed),"cache_hits":self.cache_hits.load(Ordering::Relaxed),"cache_misses":self.cache_misses.load(Ordering::Relaxed)})
    }
}

#[derive(Default)]
struct Cache {
    values: HashMap<(usize, u64), (Arc<Block>, u64)>,
    clock: u64,
    bytes: usize,
    peak_bytes: usize,
}
impl Cache {
    fn get(&mut self, key: (usize, u64)) -> Option<Arc<Block>> {
        self.clock += 1;
        let clock = self.clock;
        self.values.get_mut(&key).map(|(block, used)| {
            *used = clock;
            block.clone()
        })
    }
    fn insert(&mut self, key: (usize, u64), block: Arc<Block>) {
        let bytes = block.retained_bytes();
        if bytes > CACHE_BYTES {
            return;
        }
        if let Some((prior, _)) = self.values.remove(&key) {
            self.bytes -= prior.retained_bytes();
        }
        while !self.values.is_empty()
            && (self.bytes + bytes > CACHE_BYTES || self.values.len() >= CACHE_ENTRIES)
        {
            let oldest = self
                .values
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
                .expect("nonempty cache");
            if let Some((prior, _)) = self.values.remove(&oldest) {
                self.bytes -= prior.retained_bytes();
            }
        }
        self.clock += 1;
        self.bytes += bytes;
        self.peak_bytes = self.peak_bytes.max(self.bytes);
        self.values.insert(key, (block, self.clock));
    }
}

pub struct Reader {
    pub db: DatabaseConnection,
    root: PathBuf,
    spec: Spec,
    cache: Mutex<Cache>,
    permits: tokio::sync::Semaphore,
    pub reads: Reads,
}

pub enum Page {
    Bytes(Vec<u8>),
    Events(Vec<EventRecord>),
}
impl Page {
    pub fn units(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.len(),
            Self::Events(records) => records.len(),
        }
    }
}

impl Reader {
    pub async fn open(root: &Path, spec: Spec) -> AnyResult<Arc<Self>> {
        Ok(Arc::new(Self {
            db: connect(root, 4).await?,
            root: root.to_owned(),
            spec,
            cache: Mutex::new(Cache::default()),
            permits: tokio::sync::Semaphore::new(READ_PERMITS),
            reads: Reads::default(),
        }))
    }
    pub fn cache_summary(&self) -> Json {
        let cache = self.cache.lock().expect("cache poisoned");
        json!({"bytes":cache.bytes,"peak_bytes":cache.peak_bytes,"entries":cache.values.len(),"byte_limit":CACHE_BYTES,"entry_limit":CACHE_ENTRIES})
    }
    async fn load(
        &self,
        source: usize,
        start: u64,
        end: u64,
        length: usize,
        digest: [u8; 32],
    ) -> AnyResult<Arc<Block>> {
        let cached = self
            .cache
            .lock()
            .expect("cache poisoned")
            .get((source, start));
        if let Some(block) = cached {
            if block.end() != end || block.bytes().len() != length || *block.digest() != digest {
                return Err("immutable cache metadata mismatch".into());
            }
            self.reads.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(block);
        }
        self.reads.cache_misses.fetch_add(1, Ordering::Relaxed);
        self.reads.payload_queries.fetch_add(1, Ordering::Relaxed);
        let row = self
            .db
            .query_one(bound(
                "SELECT body,file,file_offset FROM blocks WHERE source=? AND logical_start=?",
                vec![(source as i64).into(), (start as i64).into()],
            ))
            .await?
            .ok_or("missing indexed block")?;
        let bytes = if let Some(bytes) = row.try_get::<Option<Vec<u8>>>("", "body")? {
            bytes
        } else {
            let name: String = row.try_get("", "file")?;
            // The only legal filename is a content-addressed merged batch.
            if name.len() != 71
                || !name.ends_with(".blocks")
                || !name[..64].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("invalid experimental merged-file reference".into());
            }
            let offset: i64 = row.try_get("", "file_offset")?;
            if offset < 0 {
                return Err("negative merged file offset".into());
            }
            let mut file = tokio::fs::File::open(self.root.join(name)).await?;
            if offset as u64 + length as u64 > file.metadata().await?.len() {
                return Err("merged file reference exceeds file".into());
            }
            file.seek(std::io::SeekFrom::Start(offset as u64)).await?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await?;
            self.reads
                .file_payload_read_bytes
                .fetch_add(length as u64, Ordering::Relaxed);
            bytes
        };
        if bytes.len() != length {
            return Err("encoded block length mismatch".into());
        }
        self.reads
            .fetched_payload_bytes
            .fetch_add(length as u64, Ordering::Relaxed);
        let block = Arc::new(Block::from_stored(&self.spec, start, end, bytes, digest)?);
        self.cache
            .lock()
            .expect("cache poisoned")
            .insert((source, start), block.clone());
        Ok(block)
    }
    /// Seek metadata, then bounded overlapping metadata. Payload is fetched on
    /// cache miss only. SQLite rows are released before filesystem reads or
    /// event decoding; no transaction spans observer work or sink delays.
    pub async fn page(
        &self,
        source: usize,
        after: u64,
        through: u64,
        limit: usize,
    ) -> AnyResult<Page> {
        if limit == 0 || limit > 128 * 1024 || after > through {
            return Err("invalid experimental range".into());
        }
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| "comparison reader closed")?;
        let mut bytes = Vec::new();
        let mut records = Vec::new();
        if after == through {
            return Ok(if self.spec.workload.event_kind().is_some() {
                Page::Events(records)
            } else {
                Page::Bytes(bytes)
            });
        }
        self.reads.metadata_queries.fetch_add(1, Ordering::Relaxed);
        let first = self.db.query_one(bound("SELECT logical_start FROM blocks WHERE source=? AND logical_start<=? ORDER BY logical_start DESC LIMIT 1", vec![(source as i64).into(), (after as i64).into()])).await?.ok_or("range has no first block")?;
        let first: i64 = first.try_get("", "logical_start")?;
        let end = through.min(after.checked_add(limit as u64).ok_or("range overflow")?);
        self.reads.metadata_queries.fetch_add(1, Ordering::Relaxed);
        let rows = self.db.query_all(bound("SELECT logical_start,logical_end,encoded_bytes,digest FROM blocks WHERE source=? AND logical_start>=? AND logical_start<? ORDER BY logical_start LIMIT 256", vec![(source as i64).into(), first.into(), (end as i64).into()])).await?;
        self.reads
            .metadata_rows
            .fetch_add(rows.len() as u64, Ordering::Relaxed);
        let mut cursor = after;
        for row in rows {
            let start = row.try_get::<i64>("", "logical_start")? as u64;
            let block_end = row.try_get::<i64>("", "logical_end")? as u64;
            let length: i64 = row.try_get("", "encoded_bytes")?;
            if !(1..=17 * 1024 * 1024).contains(&length) || cursor < start || cursor >= block_end {
                return Err("invalid bounded block range".into());
            }
            let digest: [u8; 32] = row
                .try_get::<Vec<u8>>("", "digest")?
                .try_into()
                .map_err(|_| "invalid digest length")?;
            let block = self
                .load(source, start, block_end, length as usize, digest)
                .await?;
            match &*block {
                Block::Events(block) => {
                    for record in block.iter_after(cursor).take((end - cursor) as usize) {
                        let record = record?;
                        if record.sequence != cursor + 1 {
                            return Err("indexed event page has a gap".into());
                        }
                        cursor = record.sequence;
                        self.reads.decoded_events.fetch_add(1, Ordering::Relaxed);
                        records.push(record);
                    }
                }
                _ => {
                    let last = end.min(block.end());
                    bytes.extend_from_slice(
                        &block.bytes()[(cursor - start) as usize..(last - start) as usize],
                    );
                    cursor = last;
                }
            }
            if cursor == end {
                break;
            }
        }
        if cursor != end {
            return Err("indexed page returned an incomplete committed range".into());
        }
        Ok(if self.spec.workload.event_kind().is_some() {
            Page::Events(records)
        } else {
            Page::Bytes(bytes)
        })
    }
}

pub struct Store {
    pub db: DatabaseConnection,
    pub reader: Arc<Reader>,
    pub root: PathBuf,
    pub adapter: Adapter,
    pub writes: Writes,
    pub sources: usize,
}

#[derive(Serialize)]
pub struct CommitTiming {
    pub total_ms: f64,
    pub prepare_ms: f64,
    pub sqlite_transaction_ms: f64,
    pub retry: bool,
}

impl Store {
    pub async fn open(
        root: &Path,
        token: &str,
        adapter: Adapter,
        spec: Spec,
        sources: usize,
        initialize: bool,
    ) -> AnyResult<Self> {
        super::owned(root, token)?;
        let db = connect(root, 1).await?;
        if initialize {
            let txn = db.begin().await?;
            for declaration in [
                "CREATE TABLE sources (source INTEGER PRIMARY KEY,kind TEXT NOT NULL,head INTEGER NOT NULL CHECK(head>=0))",
                "CREATE TABLE blocks (source INTEGER NOT NULL REFERENCES sources(source),logical_start INTEGER NOT NULL CHECK(logical_start>=0),logical_end INTEGER NOT NULL CHECK(logical_end>logical_start),encoded_bytes INTEGER NOT NULL CHECK(encoded_bytes>0),digest BLOB NOT NULL CHECK(length(digest)=32),body BLOB,file TEXT,file_offset INTEGER NOT NULL CHECK(file_offset>=0),PRIMARY KEY(source,logical_start),CHECK((body IS NULL)!=(file IS NULL)),CHECK(body IS NULL OR length(body)=encoded_bytes)) WITHOUT ROWID",
                "CREATE TABLE receipts (batch TEXT PRIMARY KEY,digest BLOB NOT NULL CHECK(length(digest)=32),blocks INTEGER NOT NULL,payload_bytes INTEGER NOT NULL) WITHOUT ROWID",
            ] {
                txn.execute(sql(declaration)).await?;
            }
            for source in 0..sources {
                txn.execute(bound(
                    "INSERT INTO sources VALUES (?,?,0)",
                    vec![(source as i64).into(), spec.workload.name().into()],
                ))
                .await?;
            }
            txn.commit().await?;
            db.execute(sql("PRAGMA wal_checkpoint(TRUNCATE)")).await?;
        }
        let reader = Reader::open(root, spec).await?;
        Ok(Self {
            db,
            reader,
            root: root.to_owned(),
            adapter,
            writes: Writes::default(),
            sources,
        })
    }
    pub async fn heads(&self) -> AnyResult<Vec<u64>> {
        let rows = self
            .db
            .query_all(sql("SELECT source,head FROM sources ORDER BY source"))
            .await?;
        if rows.len() != self.sources {
            return Err("source count mismatch".into());
        }
        rows.into_iter()
            .enumerate()
            .map(|(source, row)| {
                if row.try_get::<i64>("", "source")? as usize != source {
                    return Err("source identity mismatch".into());
                }
                Ok(row.try_get::<i64>("", "head")? as u64)
            })
            .collect()
    }
    pub async fn commit(
        &mut self,
        items: &[Item],
        fault: Option<Fault>,
    ) -> AnyResult<CommitTiming> {
        if items.is_empty() {
            return Err("empty experimental commit".into());
        }
        let started = Instant::now();
        let digest = batch_digest(items);
        let batch = hex::encode(digest);
        let bytes: usize = items.iter().map(|item| item.block.bytes().len()).sum();
        if let Some(receipt) = self
            .db
            .query_one(bound(
                "SELECT digest,blocks,payload_bytes FROM receipts WHERE batch=?",
                vec![batch.clone().into()],
            ))
            .await?
        {
            if receipt.try_get::<Vec<u8>>("", "digest")? != digest
                || receipt.try_get::<i64>("", "blocks")? as usize != items.len()
                || receipt.try_get::<i64>("", "payload_bytes")? as usize != bytes
            {
                return Err("receipt conflict".into());
            }
            self.writes.reference_receipt_retries += 1;
            return Ok(CommitTiming {
                total_ms: started.elapsed().as_secs_f64() * 1000.0,
                prepare_ms: 0.0,
                sqlite_transaction_ms: 0.0,
                retry: true,
            });
        }
        let prepare = Instant::now();
        let filename = format!("{batch}.blocks");
        if self.adapter == Adapter::Files {
            let path = self.root.join(&filename);
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(mut file) => {
                    for item in items {
                        file.write_all(item.block.bytes()).await?;
                        self.writes.file_payload_write_bytes += item.block.bytes().len() as u64;
                    }
                    file.sync_all().await?;
                    self.writes.file_fsync_calls += 1;
                    self.writes.merged_files_created += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let mut file = tokio::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&path)
                        .await?;
                    if file.metadata().await?.len() != bytes as u64 {
                        return Err("immutable merged file length conflict".into());
                    }
                    for item in items {
                        let mut actual = vec![0; item.block.bytes().len()];
                        file.read_exact(&mut actual).await?;
                        if actual != item.block.bytes() {
                            return Err("immutable merged file content conflict".into());
                        }
                    }
                    file.sync_all().await?;
                    self.writes.file_fsync_calls += 1;
                }
                Err(error) => return Err(error.into()),
            }
            let directory = self.root.clone();
            tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
                .await??;
            self.writes.directory_fsync_calls += 1;
        }
        let prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
        pause_at(fault, Fault::Prepared).await?;
        let transaction = Instant::now();
        let txn = self.db.begin().await?;
        let mut offset = 0usize;
        for item in items {
            let head: i64 = txn
                .query_one(bound(
                    "SELECT head FROM sources WHERE source=?",
                    vec![(item.source as i64).into()],
                ))
                .await?
                .ok_or("missing source")?
                .try_get("", "head")?;
            if head as u64 != item.block.start() {
                return Err("commit would overlap or leave a gap".into());
            }
            let body = (self.adapter == Adapter::Sqlite).then(|| item.block.bytes().to_vec());
            let file = (self.adapter == Adapter::Files).then(|| filename.clone());
            txn.execute(bound(
                "INSERT INTO blocks VALUES (?,?,?,?,?,?,?,?)",
                vec![
                    (item.source as i64).into(),
                    (item.block.start() as i64).into(),
                    (item.block.end() as i64).into(),
                    (item.block.bytes().len() as i64).into(),
                    item.block.digest().to_vec().into(),
                    body.into(),
                    file.into(),
                    (offset as i64).into(),
                ],
            ))
            .await?;
            txn.execute(bound(
                "UPDATE sources SET head=? WHERE source=?",
                vec![
                    (item.block.end() as i64).into(),
                    (item.source as i64).into(),
                ],
            ))
            .await?;
            offset += item.block.bytes().len();
        }
        txn.execute(bound(
            "INSERT INTO receipts VALUES (?,?,?,?)",
            vec![
                batch.into(),
                digest.to_vec().into(),
                (items.len() as i64).into(),
                (bytes as i64).into(),
            ],
        ))
        .await?;
        pause_at(fault, Fault::Transaction).await?;
        txn.commit().await?;
        let sqlite_transaction_ms = transaction.elapsed().as_secs_f64() * 1000.0;
        self.writes.sqlite_transactions += 1;
        self.writes.committed_blocks += items.len() as u64;
        self.writes.committed_encoded_bytes += bytes as u64;
        self.writes.wal_peak_file_bytes = self.writes.wal_peak_file_bytes.max(
            std::fs::metadata(self.root.join("blocks.db-wal")).map_or(0, |metadata| metadata.len()),
        );
        pause_at(fault, Fault::Committed).await?;
        Ok(CommitTiming {
            total_ms: started.elapsed().as_secs_f64() * 1000.0,
            prepare_ms,
            sqlite_transaction_ms,
            retry: false,
        })
    }
    pub async fn storage(&self) -> AnyResult<Json> {
        let page_size: i64 = self
            .db
            .query_one(sql("PRAGMA page_size"))
            .await?
            .ok_or("missing page size")?
            .try_get("", "page_size")?;
        let pages: i64 = self
            .db
            .query_one(sql("PRAGMA page_count"))
            .await?
            .ok_or("missing page count")?
            .try_get("", "page_count")?;
        let free: i64 = self
            .db
            .query_one(sql("PRAGMA freelist_count"))
            .await?
            .ok_or("missing freelist count")?
            .try_get("", "freelist_count")?;
        let mut files = 0u64;
        let mut tree_bytes = 0u64;
        let mut merged_bytes = 0u64;
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == "blocks.db-shm" || name == super::MARKER {
                continue;
            }
            files += 1;
            tree_bytes += entry.metadata()?.len();
            if name.to_string_lossy().ends_with(".blocks") {
                merged_bytes += entry.metadata()?.len();
            }
        }
        Ok(
            json!({"sqlite_page_bytes":pages * page_size,"sqlite_page_size":page_size,"sqlite_pages":pages,"sqlite_free_pages":free,"files_excluding_shm_marker":files,"file_bytes_excluding_shm_marker":tree_bytes,"merged_payload_file_bytes":merged_bytes,"wal_file_bytes":std::fs::metadata(self.root.join("blocks.db-wal")).map_or(0, |metadata| metadata.len())}),
        )
    }
    pub async fn seek_plan(&self) -> AnyResult<Vec<String>> {
        self.db.query_all(sql("EXPLAIN QUERY PLAN SELECT logical_start FROM blocks WHERE source=0 AND logical_start<=65536 ORDER BY logical_start DESC LIMIT 1")).await?.into_iter().map(|row| Ok(row.try_get("", "detail")?)).collect()
    }
    pub async fn close(self) -> AnyResult<()> {
        self.reader.db.clone().close().await?;
        self.db.close().await?;
        Ok(())
    }
}
