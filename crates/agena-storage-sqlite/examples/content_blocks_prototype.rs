//! P1 text-block comparison. Both adapters commit the same indexed blocks and
//! heads in a FULL/WAL SQLite transaction. The file adapter syncs immutable
//! files and their directory before publishing references in that transaction.
//! This is a storage prototype, not production wiring or a UI benchmark.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use agena_storage::backend::content_blocks::{
    DEFAULT_TEXT_BLOCK_BYTES, ImmutableTextBlock, TextBlockAssembler,
};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
    TransactionTrait, Value,
};
use serde_json::json;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const PAYLOAD_BYTES: usize = 8192 * 17;
const CHECKPOINT_BYTES: usize = 64 * 1024;
const SEED: u64 = 0x2026_1018_534b_3031;

#[derive(Debug, Clone, Copy)]
enum Adapter {
    Sqlite,
    Files,
}

struct Store {
    db: DatabaseConnection,
    root: PathBuf,
    adapter: Adapter,
    transactions: usize,
    file_sync_calls: usize,
    file_payload_write_bytes: usize,
    wal_peak_bytes: u64,
}

async fn connect(root: &Path) -> Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(format!(
        "sqlite://{}?mode=rwc",
        root.join("blocks.db").display()
    ));
    options
        .max_connections(1)
        .sqlx_logging(false)
        .map_sqlx_sqlite_opts(|opts| {
            use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
            opts.journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Full)
                .foreign_keys(true)
                .pragma("cache_size", "-8192")
                .pragma("mmap_size", "0")
        });
    Ok(Database::connect(options).await?)
}

impl Store {
    async fn open(root: &Path, adapter: Adapter, sources: usize, initialize: bool) -> Result<Self> {
        let db = connect(root).await?;
        if initialize {
            for sql in [
                "CREATE TABLE heads (source INTEGER PRIMARY KEY, head INTEGER NOT NULL CHECK(head>=0))",
                "CREATE TABLE blocks (source INTEGER NOT NULL REFERENCES heads(source), logical_start INTEGER NOT NULL CHECK(logical_start>=0), logical_end INTEGER NOT NULL CHECK(logical_end>logical_start), digest BLOB NOT NULL CHECK(length(digest)=32), body BLOB, file TEXT, PRIMARY KEY(source,logical_start), CHECK((body IS NULL)!=(file IS NULL)), CHECK(body IS NULL OR length(body)=logical_end-logical_start)) WITHOUT ROWID",
            ] {
                db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                    .await?;
            }
            for source in 0..sources {
                db.execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO heads VALUES (?,0)",
                    [Value::from(source as i64)],
                ))
                .await?;
            }
        }
        let mode: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA synchronous",
            ))
            .await?
            .unwrap()
            .try_get("", "synchronous")?;
        if mode != 2 {
            return Err("prototype did not retain FULL synchronous mode".into());
        }
        Ok(Self {
            db,
            root: root.to_owned(),
            adapter,
            transactions: 0,
            file_sync_calls: 0,
            file_payload_write_bytes: 0,
            wal_peak_bytes: 0,
        })
    }

    async fn commit(&mut self, blocks: &[(usize, ImmutableTextBlock)]) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let mut prepared = Vec::with_capacity(blocks.len());
        // No database transaction is held across filesystem preparation.
        for (source, block) in blocks {
            let file = match self.adapter {
                Adapter::Sqlite => None,
                Adapter::Files => {
                    let name = format!("source-{source}-{:020}.block", block.start());
                    let path = self.root.join(&name);
                    match tokio::fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&path)
                        .await
                    {
                        Ok(mut file) => {
                            use tokio::io::AsyncWriteExt;
                            file.write_all(block.bytes()).await?;
                            file.sync_all().await?;
                            self.file_sync_calls += 1;
                            self.file_payload_write_bytes += block.bytes().len();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            if tokio::fs::read(path).await? != block.bytes() {
                                return Err("immutable file conflict".into());
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                    Some(name)
                }
            };
            prepared.push(file);
        }
        if matches!(self.adapter, Adapter::Files) {
            // The prototype currently targets the Unix durability experiment;
            // production platform-specific publication remains a P2 concern.
            let directory = self.root.clone();
            tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
                .await??;
            self.file_sync_calls += 1;
        }
        let txn = self.db.begin().await?;
        for ((source, block), file) in blocks.iter().zip(prepared) {
            let head: i64 = txn
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "SELECT head FROM heads WHERE source=?",
                    [Value::from(*source as i64)],
                ))
                .await?
                .ok_or("missing source")?
                .try_get("", "head")?;
            if head as u64 >= block.end() {
                let row = txn
                    .query_one(Statement::from_sql_and_values(
                        DatabaseBackend::Sqlite,
                        "SELECT logical_end,digest FROM blocks WHERE source=? AND logical_start=?",
                        [
                            Value::from(*source as i64),
                            Value::from(block.start() as i64),
                        ],
                    ))
                    .await?
                    .ok_or("retry has no committed block")?;
                if row.try_get::<i64>("", "logical_end")? as u64 != block.end()
                    || row.try_get::<Vec<u8>>("", "digest")? != block.digest()
                {
                    return Err("conflicting block retry".into());
                }
                continue;
            }
            if head as u64 != block.start() {
                return Err("block append would create a gap".into());
            }
            let body = matches!(self.adapter, Adapter::Sqlite).then(|| block.bytes().to_vec());
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO blocks VALUES (?,?,?,?,?,?)",
                vec![
                    (*source as i64).into(),
                    (block.start() as i64).into(),
                    (block.end() as i64).into(),
                    block.digest().to_vec().into(),
                    body.into(),
                    file.into(),
                ],
            ))
            .await?;
            txn.execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE heads SET head=? WHERE source=?",
                [(block.end() as i64).into(), (*source as i64).into()],
            ))
            .await?;
        }
        txn.commit().await?;
        self.transactions += 1;
        self.wal_peak_bytes = self
            .wal_peak_bytes
            .max(std::fs::metadata(self.root.join("blocks.db-wal")).map_or(0, |m| m.len()));
        Ok(())
    }

    /// Two indexed queries: seek the block containing start, then fetch only
    /// overlapping blocks. No full source loading or directory enumeration.
    async fn read(&self, source: usize, start: u64, max_bytes: usize) -> Result<Vec<u8>> {
        let end = start
            .checked_add(max_bytes as u64)
            .ok_or("range overflow")?;
        let first=self.db.query_one(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT logical_start FROM blocks WHERE source=? AND logical_start<=? ORDER BY logical_start DESC LIMIT 1",
            [Value::from(source as i64),Value::from(start as i64)])).await?;
        let Some(first) = first else {
            return Ok(Vec::new());
        };
        let first: i64 = first.try_get("", "logical_start")?;
        let rows=self.db.query_all(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "SELECT logical_start,logical_end,digest,body,file FROM blocks WHERE source=? AND logical_start>=? AND logical_start<? ORDER BY logical_start LIMIT 64",
            [Value::from(source as i64),Value::from(first),Value::from(end as i64)])).await?;
        let mut output = Vec::new();
        let mut expected = start;
        for row in rows {
            let block_start = row.try_get::<i64>("", "logical_start")? as u64;
            let block_end = row.try_get::<i64>("", "logical_end")? as u64;
            let bytes = match self.adapter {
                Adapter::Sqlite => row.try_get::<Vec<u8>>("", "body")?,
                Adapter::Files => {
                    tokio::fs::read(self.root.join(row.try_get::<String>("", "file")?)).await?
                }
            };
            let digest: [u8; 32] = row
                .try_get::<Vec<u8>>("", "digest")?
                .try_into()
                .map_err(|_| "invalid digest length")?;
            let block = ImmutableTextBlock::from_stored(block_start, block_end, bytes, digest)?;
            let last = end.min(block.end());
            if expected < block.start() {
                return Err("range read contains a gap".into());
            }
            if last > expected {
                output.extend_from_slice(block.range(expected, last)?);
                expected = last;
            }
        }
        Ok(output)
    }
}

fn payload() -> String {
    let mut seed = SEED;
    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \n";
    (0..PAYLOAD_BYTES)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            alphabet[seed as usize % alphabet.len()] as char
        })
        .collect()
}

fn tree_bytes(root: &Path) -> std::io::Result<u64> {
    std::fs::read_dir(root)?.try_fold(0, |sum, entry| {
        let entry = entry?;
        Ok(sum
            + if entry.file_name() == "blocks.db-shm" {
                0
            } else {
                entry.metadata()?.len()
            })
    })
}

async fn measure(
    adapter: Adapter,
    packets: usize,
    sources: usize,
    observers: usize,
    repeat: usize,
) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::open(dir.path(), adapter, sources, true).await?;
    store
        .db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA wal_checkpoint(TRUNCATE)",
        ))
        .await?;
    let empty_bytes = tree_bytes(dir.path())?;
    let body = payload();
    let mut assemblers: Vec<_> = (0..sources)
        .map(|_| TextBlockAssembler::new(0, DEFAULT_TEXT_BLOCK_BYTES))
        .collect::<std::result::Result<_, _>>()?;
    let mut hashes = vec![vec![0u64; observers]; sources];
    let mut pending = Vec::new();
    let mut pending_bytes = 0;
    let mut peak_pending_bytes = 0;
    let mut committed_blocks = 0;
    let mut commit_ms = Vec::new();
    let start = Instant::now();
    for packet in body.as_bytes().chunks(body.len() / packets) {
        let text = std::str::from_utf8(packet)?;
        for source in 0..sources {
            // Realtime observers receive bytes before storage is checkpointed.
            for hash in &mut hashes[source] {
                for byte in packet {
                    *hash = hash.wrapping_mul(131).wrapping_add(u64::from(*byte));
                }
            }
            let mut rest = text;
            while !rest.is_empty() {
                let push = assemblers[source].push(rest)?;
                rest = &rest[push.consumed_bytes..];
                if let Some(block) = push.block {
                    pending_bytes += block.bytes().len();
                    pending.push((source, block));
                }
            }
        }
        peak_pending_bytes = peak_pending_bytes.max(pending_bytes);
        if pending_bytes >= CHECKPOINT_BYTES {
            let now = Instant::now();
            store.commit(&pending).await?;
            commit_ms.push(now.elapsed().as_secs_f64() * 1000.0);
            committed_blocks += pending.len();
            pending.clear();
            pending_bytes = 0;
        }
    }
    for (source, assembler) in assemblers.iter_mut().enumerate() {
        if let Some(block) = assembler.checkpoint() {
            pending.push((source, block));
        }
    }
    let now = Instant::now();
    store.commit(&pending).await?;
    commit_ms.push(now.elapsed().as_secs_f64() * 1000.0);
    committed_blocks += pending.len();
    // Replaying the exact committed batch is idempotent after response loss.
    store.commit(&pending).await?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let expected_hash = body.bytes().fold(0u64, |hash, byte| {
        hash.wrapping_mul(131).wrapping_add(u64::from(byte))
    });
    if hashes.iter().flatten().any(|h| *h != expected_hash) {
        return Err("observer content mismatch".into());
    }
    let transactions = store.transactions;
    let file_sync_calls = store.file_sync_calls;
    let file_payload_write_bytes = store.file_payload_write_bytes;
    let wal_peak_bytes = store.wal_peak_bytes;
    store.db.close().await?;
    // Reopen through a new pool, then seek across block boundaries. This is a
    // reopen test, not a substitute for P2's kill/fault-injection matrix.
    let reopened = Store::open(dir.path(), adapter, sources, false).await?;
    for source in 0..sources {
        let mut cursor = 0;
        while cursor < body.len() {
            let bytes = reopened.read(source, cursor as u64, 65536).await?;
            if bytes.is_empty() || bytes != body.as_bytes()[cursor..(cursor + bytes.len())] {
                return Err("reopened content mismatch".into());
            }
            cursor += bytes.len();
        }
        for offset in [1, 17, 32767, 32768, 65535, PAYLOAD_BYTES - 7] {
            let bytes = reopened.read(source, offset as u64, 19).await?;
            if bytes != body.as_bytes()[offset..(offset + 19).min(PAYLOAD_BYTES)] {
                return Err("indexed range mismatch".into());
            }
        }
    }
    let plans=reopened.db.query_all(Statement::from_string(DatabaseBackend::Sqlite,
        "EXPLAIN QUERY PLAN SELECT logical_start FROM blocks WHERE source=0 AND logical_start<=65536 ORDER BY logical_start DESC LIMIT 1")).await?;
    let plan: Vec<String> = plans
        .into_iter()
        .map(|r| r.try_get("", "detail"))
        .collect::<std::result::Result<_, _>>()?;
    reopened.db.close().await?;
    commit_ms.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({"phase":"P1_text_prototype_only","adapter":format!("{adapter:?}"),"repeat":repeat,
        "payload_bytes_per_source":body.len(),"append_packets_per_source":packets,"sources":sources,"observers":observers,
        "equivalent":true,"restart_range_reads_equivalent":true,"idempotent_commit_verified":true,
        "block_bytes":DEFAULT_TEXT_BLOCK_BYTES,"checkpoint_bytes":CHECKPOINT_BYTES,"committed_blocks":committed_blocks,
        "transactions_including_receipt_retry":transactions,"sqlite_synchronous":"FULL","sqlite_connections":1,"sqlite_cache_kib":8192,
        "file_sync_calls_including_directory":file_sync_calls,"file_payload_write_bytes":file_payload_write_bytes,
        "sqlite_sync_calls":null,"cumulative_os_write_bytes":null,"wal_peak_file_bytes":wal_peak_bytes,
        "empty_file_bytes_excluding_shm":empty_bytes,"final_file_bytes_excluding_shm":tree_bytes(dir.path())?,
        "peak_pending_block_payload_bytes":peak_pending_bytes,"elapsed_ms":elapsed_ms,
        "checkpoint_p50_ms":commit_ms[commit_ms.len()/2],"checkpoint_p95_ms":commit_ms[(commit_ms.len()*95).div_ceil(100)-1],
        "range_seek_plan":plan,"remaining":"log/structured/PTY codecs, timed checkpoints, real-process faults, OS I/O and RSS; not production acceptance"})
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let sources = args.first().map_or(Ok(1), |s| s.parse::<usize>())?;
    let observers = args.get(1).map_or(Ok(1), |s| s.parse::<usize>())?;
    if args.len() > 2 || ![1, 8, 32].contains(&sources) || ![1, 8].contains(&observers) {
        return Err("usage: content_blocks_prototype [1|8|32 sources] [1|8 observers]".into());
    }
    for repeat in 1..=3 {
        for adapter in [Adapter::Sqlite, Adapter::Files] {
            for packets in [8192, 4] {
                measure(adapter, packets, sources, observers, repeat).await?;
            }
        }
    }
    Ok(())
}
