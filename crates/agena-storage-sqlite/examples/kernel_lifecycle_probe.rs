//! Reproducible B05/B06/B07 probes through the actual session kernel.
//!
//! Run in release mode after implementation is complete:
//!   kernel_lifecycle_probe b05 [10,100,500]
//!   kernel_lifecycle_probe b06 [100000 entries] [100 branches] [100 depth]
//!   kernel_lifecycle_probe b07 [256 log MiB] [32 mixed MiB] [4096 PTY frames] [4096 document events]
//! Smoke sizes: b05 3,10; b06 1000 4 4; b07 1 1 32 32.
//! Every writable path is owned by a TempDir. B06 generation exits in its own
//! process before measurement, so fixture allocations cannot inflate its RSS.

use std::{
    error::Error,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{KernelMetricsSnapshot, SqliteKernel, open_kernel_runtime};
use sea_orm::sqlx::{
    self, Row,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use serde_json::{Value, json};

#[path = "support/lifecycle_forks.rs"]
mod forks;
#[path = "support/kernel_resources.rs"]
mod resources;
#[path = "support/lifecycle_rounds.rs"]
mod rounds;
#[path = "support/lifecycle_streams.rs"]
mod streams;

type AnyResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const MARKER: &str = "owned-kernel-lifecycle-probe";
const PACKET_BYTES: usize = 64 * 1024;

fn emit(value: Value) -> AnyResult<()> {
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

fn owned_root() -> AnyResult<tempfile::TempDir> {
    let executable = std::env::current_exe()?;
    let target = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent);
    let parent = std::env::var_os("AGENA_KERNEL_PROBE_TEMP_PARENT").map(PathBuf::from);
    let root = match parent.as_deref().or(target) {
        Some(parent) => tempfile::Builder::new()
            .prefix("agena-kernel-lifecycle-")
            .tempdir_in(parent)?,
        None => tempfile::Builder::new()
            .prefix("agena-kernel-lifecycle-")
            .tempdir()?,
    };
    std::fs::write(root.path().join(MARKER), RequestId::new().to_string())?;
    Ok(root)
}

fn check_owned(root: &Path, token: &str) -> AnyResult<()> {
    if std::fs::read_to_string(root.join(MARKER))? != token {
        return Err("history fixture requires its parent's temporary ownership marker".into());
    }
    Ok(())
}

fn database_url(root: &Path) -> String {
    format!("sqlite://{}?mode=rwc", root.join("kernel.db").display())
}

async fn inspect(root: &Path) -> AnyResult<sqlx::SqlitePool> {
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str(&database_url(root))?
                .read_only(true)
                .pragma("cache_size", "-2048")
                .pragma("mmap_size", "0"),
        )
        .await?)
}

fn ascii(bytes: usize, seed: u64) -> String {
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_-";
    let mut state = seed | 1;
    (0..bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            alphabet[(state & 63) as usize] as char
        })
        .collect()
}

fn io_delta(before: Option<resources::Usage>, after: Option<resources::Usage>) -> Value {
    match (before, after) {
        (Some(before), Some(after)) => json!({
            "disk_read_bytes": after.disk_read_bytes.saturating_sub(before.disk_read_bytes),
            "disk_written_bytes": after.disk_written_bytes.saturating_sub(before.disk_written_bytes),
            "darwin_ri_logical_writes": after.darwin_ri_logical_writes.saturating_sub(before.darwin_ri_logical_writes),
            "resident_bytes_signed_delta": i128::from(after.resident_bytes)-i128::from(before.resident_bytes),
        }),
        _ => Value::Null,
    }
}

fn diagnostics(kernel: &SqliteKernel) -> Value {
    json!({"counters":kernel.diagnostics.snapshot(), "read_cache":kernel.diagnostics.read_cache(), "read_budget":kernel.diagnostics.read_budget()})
}

fn metric_delta(before: &KernelMetricsSnapshot, after: &KernelMetricsSnapshot) -> AnyResult<Value> {
    let before = serde_json::to_value(before)?;
    let after = serde_json::to_value(after)?;
    Ok(Value::Object(
        after
            .as_object()
            .ok_or("metrics are not an object")?
            .iter()
            .map(|(key, value)| {
                let old = before.get(key).and_then(Value::as_u64).unwrap_or(0);
                (
                    key.clone(),
                    json!(value.as_u64().unwrap_or(0).saturating_sub(old)),
                )
            })
            .collect(),
    ))
}

fn file_bytes(path: &Path) -> AnyResult<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn files(root: &Path) -> AnyResult<Value> {
    Ok(
        json!({"database_bytes":file_bytes(&root.join("kernel.db"))?, "wal_bytes":file_bytes(&root.join("kernel.db-wal"))?, "shm_bytes":file_bytes(&root.join("kernel.db-shm"))?}),
    )
}

async fn stored(pool: &sqlx::SqlitePool, root: &Path) -> AnyResult<Value> {
    let row = sqlx::query("SELECT (SELECT count(*) FROM agena_sessions) AS sessions, (SELECT count(*) FROM agena_entries) AS entries, (SELECT count(*) FROM agena_history_segments) AS segments, (SELECT count(*) FROM agena_segment_entries) AS segment_entries, (SELECT count(*) FROM agena_session_segments) AS session_edges, (SELECT count(*) FROM agena_retired_session_segments) AS retired_edges, (SELECT count(*) FROM agena_rounds) AS rounds, (SELECT COALESCE(sum(provider_state_bytes),0) FROM agena_rounds) AS provider_state_bytes, (SELECT count(*) FROM agena_round_inputs) AS round_inputs, (SELECT count(*) FROM agena_round_input_replays) AS replay_edges, (SELECT count(*) FROM agena_content_blocks) AS content_blocks, (SELECT COALESCE(sum(length(body)),0) FROM agena_content_blocks) AS content_block_bytes, (SELECT count(*) FROM agena_content_checkpoints) AS content_checkpoints, (SELECT count(*) FROM agena_content_capture_losses) AS capture_loss_windows, (SELECT count(*) FROM agena_deliveries) AS deliveries")
        .fetch_one(pool).await?;
    let mut result = serde_json::Map::new();
    for key in [
        "sessions",
        "entries",
        "segments",
        "segment_entries",
        "session_edges",
        "retired_edges",
        "rounds",
        "provider_state_bytes",
        "round_inputs",
        "replay_edges",
        "content_blocks",
        "content_block_bytes",
        "content_checkpoints",
        "capture_loss_windows",
        "deliveries",
    ] {
        result.insert(key.into(), json!(row.try_get::<i64, _>(key)?));
    }
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await?;
    let size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(pool)
        .await?;
    let free: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(pool)
        .await?;
    result.insert("page_count".into(), json!(pages));
    result.insert("page_size".into(), json!(size));
    result.insert("freelist_pages".into(), json!(free));
    result.insert("database_page_bytes".into(), json!(pages * size));
    result.insert("files".into(), files(root)?);
    Ok(Value::Object(result))
}

struct Sampler {
    active: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    available: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Sampler {
    fn start() -> Self {
        let active = Arc::new(AtomicBool::new(true));
        let peak = Arc::new(AtomicU64::new(0));
        let available = Arc::new(AtomicBool::new(false));
        let worker = {
            let active = active.clone();
            let peak = peak.clone();
            let available = available.clone();
            std::thread::spawn(move || {
                while active.load(Ordering::Relaxed) {
                    if let Some(usage) = resources::sample() {
                        available.store(true, Ordering::Relaxed);
                        peak.fetch_max(usage.resident_bytes, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };
        Self {
            active,
            peak,
            available,
            worker: Some(worker),
        }
    }
    fn summary(&self) -> Value {
        json!({"rss_peak_sampled_bytes":self.available.load(Ordering::Relaxed).then(||self.peak.load(Ordering::Relaxed)), "rss_sample_interval_ms":10})
    }
}
impl Drop for Sampler {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn start_attempt(
    kernel: &SqliteKernel,
    root: &Path,
    title: &str,
) -> AnyResult<(SessionId, ExecutionView, AttemptHandle)> {
    let workspace = kernel
        .workspaces
        .create(root.to_string_lossy().into_owned())
        .await?;
    let session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace.id, title.into())
        .await?
        .value
        .id;
    let (execution, attempt) = claim_in(kernel, session).await?;
    Ok((session, execution, attempt))
}

async fn claim_in(
    kernel: &SqliteKernel,
    session: SessionId,
) -> AnyResult<(ExecutionView, AttemptHandle)> {
    let execution = kernel
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "isolated lifecycle probe".into(),
            }]),
        )
        .await?
        .value;
    let attempt = kernel
        .ports
        .executor
        .claim(RequestId::new(), session, execution.id)
        .await?
        .value;
    Ok((execution, attempt))
}

async fn closed_checkpoint(root: &Path) -> AnyResult<Value> {
    // Called only after the kernel and every measurement connection have
    // closed. This is fixture maintenance, never a competing business writer.
    let before = files(root)?;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str(&database_url(root))?
                .synchronous(sqlx::sqlite::SqliteSynchronous::Full),
        )
        .await?;
    let now = Instant::now();
    let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(&pool)
        .await?;
    let result = json!({"elapsed_ms":now.elapsed().as_secs_f64()*1000.0, "busy":checkpoint.try_get::<i64,_>(0)?, "wal_frames":checkpoint.try_get::<i64,_>(1)?, "checkpointed_frames":checkpoint.try_get::<i64,_>(2)?, "files_before":before, "files_after":files(root)?});
    pool.close().await;
    Ok(result)
}

fn number(args: &[String], index: usize, default: usize) -> AnyResult<usize> {
    Ok(args.get(index).map_or(Ok(default), |value| value.parse())?)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> AnyResult<()> {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("b05") => rounds::run(args.get(2).map_or("10,100,500", String::as_str)).await,
        Some("b06") => forks::run(number(&args,2,100000)?, number(&args,3,100)?, number(&args,4,100)?).await,
        Some("b07") => streams::run(number(&args,2,256)?, number(&args,3,32)?, number(&args,4,4096)?, number(&args,5,4096)?).await,
        Some("--history-fixture") if args.len()==5 => emit(forks::generate(Path::new(&args[2]), &args[3], args[4].parse()?).await?),
        _ => Err("usage: kernel_lifecycle_probe b05 [10,100,500] | b06 [100000 entries] [100 branches] [100 depth] | b07 [256 log MiB] [32 mixed MiB] [4096 PTY frames] [4096 document events]".into()),
    }
}
