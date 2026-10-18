//! P1 comparison of identical immutable payloads in SQLite or merged files.
//! See support/README.md for the serial runner, durability and measurement scope.
//! No production adapter, user database, provider, network, or external tool.

use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_storage::kernel::EventProjection;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "support/block_comparison_fixture.rs"]
mod fixture;
#[path = "support/kernel_resources.rs"]
mod resources;
#[path = "support/block_comparison_store.rs"]
mod store;

use fixture::{Generator, Item, Spec, Workload};
use store::{Adapter, Fault, Page, Reader, Store};

type AnyResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
const MARKER: &str = "owned-event-blocks-comparison";

fn owned(root: &Path, token: &str) -> AnyResult<()> {
    if !root.is_absolute()
        || token.len() < 16
        || token.len() > 96
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("invalid comparison ownership identity".into());
    }
    if std::fs::symlink_metadata(root)?.file_type().is_symlink()
        || std::fs::symlink_metadata(root.join(MARKER))?
            .file_type()
            .is_symlink()
        || std::fs::read_to_string(root.join(MARKER))? != token
    {
        return Err("comparison ownership marker mismatch".into());
    }
    Ok(())
}

#[derive(Default)]
struct Latency {
    values: Vec<f64>,
}
impl Latency {
    fn add(&mut self, duration: Duration) {
        self.values.push(duration.as_secs_f64() * 1000.0);
    }
    fn ms(&mut self, milliseconds: f64) {
        self.values.push(milliseconds);
    }
    fn summary(&self) -> Value {
        if self.values.is_empty() {
            return json!({"samples":0,"p50_ms":null,"p95_ms":null,"max_ms":null});
        }
        let mut values = self.values.clone();
        values.sort_by(f64::total_cmp);
        json!({"samples":values.len(),"p50_ms":values[(values.len()*50).div_ceil(100)-1],"p95_ms":values[(values.len()*95).div_ceil(100)-1],"max_ms":values[values.len()-1],"method":"nearest-rank, per child process"})
    }
}

struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    wal_peak: Arc<AtomicU64>,
    samples: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Sampler {
    fn start(root: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let wal_peak = Arc::new(AtomicU64::new(0));
        let samples = Arc::new(AtomicU64::new(0));
        let thread = {
            let (stop, peak, wal_peak, samples) = (
                stop.clone(),
                peak.clone(),
                wal_peak.clone(),
                samples.clone(),
            );
            std::thread::spawn(move || {
                loop {
                    if let Some(usage) = resources::sample() {
                        peak.fetch_max(usage.resident_bytes, Ordering::Relaxed);
                        samples.fetch_add(1, Ordering::Relaxed);
                    }
                    wal_peak.fetch_max(
                        std::fs::metadata(root.join("blocks.db-wal")).map_or(0, |m| m.len()),
                        Ordering::Relaxed,
                    );
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };
        Self {
            stop,
            peak,
            wal_peak,
            samples,
            thread: Some(thread),
        }
    }
    fn finish(mut self) -> AnyResult<Value> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .take()
            .ok_or("missing resource sampler")?
            .join()
            .map_err(|_| "resource sampler panicked")?;
        let samples = self.samples.load(Ordering::Relaxed);
        Ok(
            json!({"rss_samples":samples,"rss_peak_sampled_bytes":(samples>0).then(|| self.peak.load(Ordering::Relaxed)),"wal_peak_sampled_file_bytes":self.wal_peak.load(Ordering::Relaxed),"sample_period_ms":10}),
        )
    }
}
impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn io_delta(before: Option<resources::Usage>, after: Option<resources::Usage>) -> Value {
    before.zip(after).map_or(Value::Null, |(before, after)| json!({
        "os_attributed_disk_read_bytes":after.disk_read_bytes.saturating_sub(before.disk_read_bytes),
        "os_attributed_disk_written_bytes":after.disk_written_bytes.saturating_sub(before.disk_written_bytes),
        "darwin_ri_logical_writes_raw_delta":after.darwin_ri_logical_writes.saturating_sub(before.darwin_ri_logical_writes),
        "scope":"this child process, proc_pid_rusage; not total device I/O or fsync counts"
    }))
}

struct Observer {
    source: usize,
    observer: usize,
    cursor: u64,
    hash: Sha256,
    projection: Option<EventProjection>,
    pages: usize,
    page_latency: Latency,
    sealed_to_observation: Latency,
}
impl Observer {
    fn new(spec: &Spec, source: usize, observer: usize) -> Self {
        Self {
            source,
            observer,
            cursor: 0,
            hash: Sha256::new(),
            projection: spec.workload.event_kind().map(EventProjection::new),
            pages: 0,
            page_latency: Latency::default(),
            sealed_to_observation: Latency::default(),
        }
    }
    fn consume(&mut self, spec: &Spec, page: Page) -> AnyResult<()> {
        if page.units() == 0 {
            return Err("observer failed to advance".into());
        }
        match page {
            Page::Bytes(bytes) => {
                if bytes != spec.bytes(self.source, self.cursor, bytes.len()) {
                    return Err("observer byte range mismatch".into());
                }
                self.hash.update(&bytes);
                self.cursor += bytes.len() as u64;
            }
            Page::Events(records) => {
                for record in records {
                    if record != spec.record(self.source, self.cursor)? {
                        return Err(
                            "observer sequence, timestamp, channel or typed payload mismatch"
                                .into(),
                        );
                    }
                    self.projection
                        .as_mut()
                        .ok_or("observer has no typed projection")?
                        .apply(&record)?;
                    self.hash.update(serde_json::to_vec(&record)?);
                    self.cursor = record.sequence;
                }
            }
        }
        self.pages += 1;
        Ok(())
    }
    fn verify(&self, expected: &Generator) -> AnyResult<()> {
        if self.cursor != expected.spec.head()
            || self.hash.clone().finalize() != expected.hash.clone().finalize()
            || self.projection.as_ref().and_then(EventProjection::document)
                != expected
                    .projection
                    .as_ref()
                    .and_then(EventProjection::document)
            || self.projection.as_ref().and_then(EventProjection::terminal)
                != expected
                    .projection
                    .as_ref()
                    .and_then(EventProjection::terminal)
        {
            return Err(
                "observer final hash, cursor, document or terminal differs from producer".into(),
            );
        }
        Ok(())
    }
}

async fn catch_up(
    reader: Arc<Reader>,
    spec: &Spec,
    observers: Vec<Observer>,
    heads: &[u64],
    sealed: Instant,
) -> AnyResult<Vec<Observer>> {
    let mut tasks = tokio::task::JoinSet::new();
    for mut observer in observers {
        let reader = reader.clone();
        let spec = spec.clone();
        let head = heads[observer.source];
        tasks.spawn(async move {
            let changed = observer.cursor < head;
            while observer.cursor < head {
                let started = Instant::now();
                let page = reader
                    .page(
                        observer.source,
                        observer.cursor,
                        head,
                        spec.workload.page_units(),
                    )
                    .await?;
                // Page latency includes query/decode; exact expected-fixture
                // comparison is measured only in the enclosing observer span.
                observer.page_latency.add(started.elapsed());
                observer.consume(&spec, page)?;
            }
            if changed {
                observer.sealed_to_observation.add(sealed.elapsed());
            }
            Ok::<_, Box<dyn Error + Send + Sync>>(observer)
        });
    }
    let mut observers = Vec::new();
    while let Some(result) = tasks.join_next().await {
        observers.push(result??);
    }
    observers.sort_by_key(|observer| (observer.source, observer.observer));
    Ok(observers)
}

fn observer_summary(observers: &[Observer]) -> Value {
    let mut page = Latency::default();
    let mut delivery = Latency::default();
    for observer in observers {
        page.values.extend_from_slice(&observer.page_latency.values);
        delivery
            .values
            .extend_from_slice(&observer.sealed_to_observation.values);
    }
    json!({"count":observers.len(),"pages":observers.iter().map(|o| o.pages).sum::<usize>(),"page_latency":page.summary(),"batch_first_sealed_to_observer_caught_up":delivery.summary(),"hashes":observers.iter().map(|o| json!({"source":o.source,"observer":o.observer,"cursor":o.cursor,"sha256":hex::encode(o.hash.clone().finalize())})).collect::<Vec<_>>()})
}

async fn check_ranges(reader: &Reader, spec: &Spec, sources: usize, head: u64) -> AnyResult<usize> {
    let mut checked = 0;
    let mut offsets = vec![
        0,
        1,
        17,
        31,
        32,
        255,
        256,
        1091,
        1092,
        32767,
        32768,
        65535,
        65536,
        131071,
        131072,
        head.saturating_sub(7),
        head,
    ];
    offsets.retain(|offset| *offset <= head);
    offsets.sort_unstable();
    offsets.dedup();
    for source in 0..sources {
        for &offset in &offsets {
            let limit = if spec.workload.event_kind().is_some() {
                19
            } else {
                257
            };
            let page = reader.page(source, offset, head, limit).await?;
            if page.units() as u64 != (head - offset).min(limit as u64) {
                return Err("reopen range has incorrect length".into());
            }
            match page {
                Page::Bytes(bytes) => {
                    if bytes != spec.bytes(source, offset, bytes.len()) {
                        return Err("reopen byte seek mismatch".into());
                    }
                }
                Page::Events(records) => {
                    for (index, record) in records.into_iter().enumerate() {
                        if record != spec.record(source, offset + index as u64)? {
                            return Err("reopen event seek mismatch".into());
                        }
                    }
                }
            }
            checked += 1;
        }
    }
    Ok(checked)
}

#[allow(clippy::too_many_arguments)]
async fn publish(
    store: &mut Store,
    spec: &Spec,
    pending: &[Item],
    observers: Vec<Observer>,
    heads: &mut [u64],
    sealed: Instant,
    commits: &mut Vec<store::CommitTiming>,
    physical_hash: &mut Sha256,
) -> AnyResult<Vec<Observer>> {
    for item in pending.iter() {
        if heads[item.source] != item.block.start() {
            return Err("shared fixture has a block gap".into());
        }
        heads[item.source] = item.block.end();
    }
    physical_hash.update(fixture::batch_digest(pending));
    commits.push(store.commit(pending, None).await?);
    let observers = catch_up(store.reader.clone(), spec, observers, heads, sealed).await?;
    // Keep the final batch available to exercise receipt replay after reopen.
    Ok(observers)
}

async fn measure(
    adapter: Adapter,
    spec: Spec,
    sources: usize,
    observer_count: usize,
) -> AnyResult<()> {
    let temporary = tempfile::tempdir()?;
    let token = format!(
        "p1-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    std::fs::write(temporary.path().join(MARKER), &token)?;
    let mut store = Store::open(
        temporary.path(),
        &token,
        adapter,
        spec.clone(),
        sources,
        true,
    )
    .await?;
    let empty = store.storage().await?;
    let initial = resources::sample();
    let sampler = Sampler::start(temporary.path().to_owned());
    let started = Instant::now();
    let mut generators = (0..sources)
        .map(|source| Generator::new(spec.clone(), source))
        .collect::<AnyResult<Vec<_>>>()?;
    let mut observers = (0..sources)
        .flat_map(|source| (0..observer_count).map(move |observer| (source, observer)))
        .map(|(source, observer)| Observer::new(&spec, source, observer))
        .collect();
    let mut heads = vec![0; sources];
    let mut pending = Vec::new();
    let mut last_batch = Vec::new();
    let mut pending_bytes = 0;
    let mut peak_pending_bytes = 0;
    let mut first_sealed = None;
    let mut commits = Vec::new();
    let mut physical_hash = Sha256::new();
    for _ in 0..spec.steps() {
        for generator in &mut generators {
            for item in generator.next_blocks()? {
                first_sealed.get_or_insert_with(Instant::now);
                pending_bytes += item.block.bytes().len();
                pending.push(item);
            }
        }
        peak_pending_bytes = peak_pending_bytes.max(pending_bytes);
        if pending_bytes >= fixture::COMMIT_BYTES {
            observers = publish(
                &mut store,
                &spec,
                &pending,
                observers,
                &mut heads,
                first_sealed.take().ok_or("batch has no seal timestamp")?,
                &mut commits,
                &mut physical_hash,
            )
            .await?;
            last_batch = std::mem::take(&mut pending);
            pending_bytes = 0;
        }
    }
    for generator in &mut generators {
        if let Some(item) = generator.finish() {
            first_sealed.get_or_insert_with(Instant::now);
            pending_bytes += item.block.bytes().len();
            pending.push(item);
        }
    }
    peak_pending_bytes = peak_pending_bytes.max(pending_bytes);
    if !pending.is_empty() {
        observers = publish(
            &mut store,
            &spec,
            &pending,
            observers,
            &mut heads,
            first_sealed
                .take()
                .ok_or("final batch has no seal timestamp")?,
            &mut commits,
            &mut physical_hash,
        )
        .await?;
        last_batch = std::mem::take(&mut pending);
    }
    if heads != vec![spec.head(); sources] || store.heads().await? != heads {
        return Err("committed heads differ from deterministic fixture".into());
    }
    for observer in &observers {
        observer.verify(&generators[observer.source])?;
    }
    let live_ms = started.elapsed().as_secs_f64() * 1000.0;
    let live_io = io_delta(initial, resources::sample());
    let live_observers = observer_summary(&observers);
    let live_reads = store.reader.reads.summary();
    let live_cache = store.reader.cache_summary();
    let before_close = store.storage().await?;
    let seek_plan = store.seek_plan().await?;
    let write_summary = serde_json::to_value(&store.writes)?;
    let commit_count = commits.len();
    let mut commit = Latency::default();
    let mut prepare = Latency::default();
    let mut sql = Latency::default();
    for timing in &commits {
        commit.ms(timing.total_ms);
        prepare.ms(timing.prepare_ms);
        sql.ms(timing.sqlite_transaction_ms);
    }
    let physical_digest = hex::encode(physical_hash.finalize());
    drop(observers);
    store.close().await?;
    let after_live_close = resources::sample();
    let restart_started = Instant::now();
    let mut reopened = Store::open(
        temporary.path(),
        &token,
        adapter,
        spec.clone(),
        sources,
        false,
    )
    .await?;
    let reopen_heads = reopened.heads().await?;
    if reopen_heads != heads {
        return Err("reopened heads differ from durable heads".into());
    }
    let retry = reopened.commit(&last_batch, None).await?;
    if !retry.retry
        || reopened.writes.file_payload_write_bytes != 0
        || reopened.writes.sqlite_transactions != 0
    {
        return Err("response-loss retry duplicated durable storage".into());
    }
    let restart_observers = (0..sources)
        .map(|source| Observer::new(&spec, source, 0))
        .collect();
    let restart_observers = catch_up(
        reopened.reader.clone(),
        &spec,
        restart_observers,
        &heads,
        Instant::now(),
    )
    .await?;
    for observer in &restart_observers {
        observer.verify(&generators[observer.source])?;
    }
    let ranges = check_ranges(&reopened.reader, &spec, sources, spec.head()).await?;
    let restart_reads = reopened.reader.reads.summary();
    let restart_cache = reopened.reader.cache_summary();
    let restart_ms = restart_started.elapsed().as_secs_f64() * 1000.0;
    reopened.close().await?;
    let after = resources::sample();
    let sampling = sampler.finish()?;
    let final_disk = {
        let closed = Store::open(
            temporary.path(),
            &token,
            adapter,
            spec.clone(),
            sources,
            false,
        )
        .await?;
        let disk = closed.storage().await?;
        closed.close().await?;
        disk
    };
    let fixture_contract = json!({
            "phase":"P1_payload_placement_comparison","adapter":adapter.name(),"workload":spec.workload.name(),"mode":if spec.smoke {"smoke"} else {"full"},"sources":sources,"observers_per_source":observer_count,
            "producer_steps_per_source":spec.steps(),"head_per_source":spec.head(),"native_unit":if spec.workload.event_kind().is_some() {"event sequence"} else {"byte"},"event_payload_bytes":generators.iter().map(|g| g.event_payload_bytes).sum::<usize>(),
            "source_proofs":generators.iter().map(Generator::summary).collect::<AnyResult<Vec<_>>>()?,"physical_batch_sha256":physical_digest,
            "all_observers_exact":true,"all_reopen_events_and_projections_exact":true,"idempotent_receipt_retry_after_reopen":true,"reopen_range_checks":ranges,
            "ordinary_block_target_bytes":fixture::BLOCK_BYTES,"blob_chunk_bytes":fixture::BLOB_CHUNK_BYTES,"commit_target_bytes":fixture::COMMIT_BYTES,"page_limit_units":spec.workload.page_units(),"peak_pending_sealed_payload_bytes":peak_pending_bytes
    });
    let commit_outcomes = json!({
            "live_elapsed_ms":live_ms,"reopen_elapsed_ms":restart_ms,"commits":commit_count,"writes":write_summary,"commit_total_latency":commit.summary(),"file_prepare_latency":prepare.summary(),"sqlite_transaction_latency":sql.summary(),
            "live_observers":live_observers,"live_reads":live_reads,"live_cache":live_cache,"restart_observers":observer_summary(&restart_observers),"restart_reads":restart_reads,"restart_cache":restart_cache
    });
    let storage_outcomes = json!({
            "storage_empty":empty,"storage_before_close":before_close,"storage_after_close_reopen":final_disk,"sampling":sampling,"io_live":live_io,"io_reopen_and_close":io_delta(after_live_close,after),"io_total_including_close_reopen":io_delta(initial,after),"range_seek_plan":seek_plan,
            "sqlite_policy":{"journal":"WAL","synchronous":"FULL","writer_connections":1,"reader_connections_max":4,"read_permits":store::READ_PERMITS,"cache_kib_per_connection":8192,"mmap_bytes":0,"wal_autocheckpoint_pages":1000},
            "scope":"prototype placement and indexed reads; deterministic round-robin producers, concurrent observers catch up after each durable batch; no production command scheduler, network, provider, UI or OS PTY scheduling; restart uses a new pool/cache in this child, not an evicted OS cache"
    });
    // Keep each macro input bounded while preserving the runner's flat JSON
    // contract. These sections describe one process, never pooled runs.
    let mut report = serde_json::Map::new();
    for section in [fixture_contract, commit_outcomes, storage_outcomes] {
        let Value::Object(fields) = section else {
            unreachable!("comparison report sections are object literals");
        };
        report.extend(fields);
    }
    println!("{}", Value::Object(report));
    Ok(())
}

async fn fault_write(
    root: &Path,
    token: &str,
    adapter: Adapter,
    spec: &Spec,
    boundary: Fault,
) -> AnyResult<()> {
    owned(root, token)?;
    if root.join("blocks.db").exists() {
        return Err("fault writer requires an empty owned directory".into());
    }
    let items = fixture::fault_blocks(spec)?;
    let mut store = Store::open(root, token, adapter, spec.clone(), 1, true).await?;
    store.commit(&items[..1], None).await?;
    store.commit(&items[1..], Some(boundary)).await?;
    Err("fault writer resumed without external SIGKILL".into())
}

async fn fault_recover(
    root: &Path,
    token: &str,
    adapter: Adapter,
    spec: &Spec,
    boundary: Fault,
) -> AnyResult<()> {
    owned(root, token)?;
    if !root.join("blocks.db").is_file() {
        return Err("fault recovery requires its existing owned database".into());
    }
    let items = fixture::fault_blocks(spec)?;
    let mut store = Store::open(root, token, adapter, spec.clone(), 1, false).await?;
    let before = store.heads().await?[0];
    let expected_before = if boundary == Fault::Committed {
        items[1].block.end()
    } else {
        items[0].block.end()
    };
    if before != expected_before {
        return Err("crash exposed a missing, partial or uncommitted head".into());
    }
    let pre_ranges = check_ranges(&store.reader, spec, 1, before).await?;
    let row = store
        .db
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA integrity_check",
        ))
        .await?
        .ok_or("missing integrity result")?;
    if row.try_get::<String>("", "integrity_check")? != "ok" {
        return Err("post-kill SQLite integrity check failed".into());
    }
    let sql_refs = store
        .db
        .query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT DISTINCT file FROM blocks WHERE file IS NOT NULL",
        ))
        .await?;
    let referenced = sql_refs
        .into_iter()
        .map(|row| row.try_get::<String>("", "file"))
        .collect::<Result<std::collections::HashSet<_>, _>>()?;
    let mut orphans = 0;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".blocks") && !referenced.contains(&name) {
            orphans += 1;
        }
    }
    let timing = store.commit(&items[1..], None).await?;
    if timing.retry != (boundary == Fault::Committed) {
        return Err("recovery receipt existence differs from commit boundary".into());
    }
    let after = store.heads().await?[0];
    if after != items[1].block.end() {
        return Err("recovered retry head mismatch".into());
    }
    let mut observer = Observer::new(spec, 0, 0);
    while observer.cursor < after {
        let page = store
            .reader
            .page(0, observer.cursor, after, spec.workload.page_units())
            .await?;
        observer.consume(spec, page)?;
    }
    // Independent typed replay of the identical immutable blocks proves the
    // final reconstructed screen/document, beyond random seek equivalence.
    let mut projection = spec.workload.event_kind().map(EventProjection::new);
    let mut expected_hash = Sha256::new();
    for item in &items {
        match &item.block {
            fixture::Block::Events(block) => {
                for record in block.iter() {
                    let record = record?;
                    projection
                        .as_mut()
                        .ok_or("missing recovery projection")?
                        .apply(&record)?;
                    expected_hash.update(serde_json::to_vec(&record)?);
                }
            }
            _ => expected_hash.update(item.block.bytes()),
        }
    }
    if observer.hash.clone().finalize() != expected_hash.finalize()
        || observer
            .projection
            .as_ref()
            .and_then(EventProjection::document)
            != projection.as_ref().and_then(EventProjection::document)
        || observer
            .projection
            .as_ref()
            .and_then(EventProjection::terminal)
            != projection.as_ref().and_then(EventProjection::terminal)
    {
        return Err("recovery event hash or final typed projection mismatch".into());
    }
    let retry = store.commit(&items[1..], None).await?;
    if !retry.retry {
        return Err("second recovery replay was not idempotent".into());
    }
    let range_checks = check_ranges(&store.reader, spec, 1, after).await?;
    let writes = serde_json::to_value(&store.writes)?;
    store.close().await?;
    println!(
        "{}",
        json!({"phase":"P1_process_kill_recovery","adapter":adapter.name(),"workload":spec.workload.name(),"boundary":boundary.name(),"head_before_replay":before,"head_after_replay":after,"baseline_block_end":items[0].block.end(),"second_block_end":items[1].block.end(),"orphan_merged_files_before_retry":orphans,"existing_prepared_file_reused":adapter == Adapter::Files && boundary != Fault::Committed && writes["file_payload_write_bytes"] == 0,"range_checks_before_replay":pre_ranges,"range_checks_after_replay":range_checks,"all_events_and_projections_exact":true,"receipt_retry_exact":true,"sqlite_integrity":"ok","recovery_writes":writes,"scope":"external SIGKILL at explicit protocol boundaries; process crash, not device power-loss or arbitrary fsync fault injection"})
    );
    Ok(())
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("measure") if args.len() == 6 => {
            let adapter = Adapter::parse(&args[1])?;
            let workload = Workload::parse(&args[2])?;
            let sources: usize = args[3].parse()?;
            let observers: usize = args[4].parse()?;
            if ![1, 8].contains(&sources) || ![1, 8].contains(&observers) || !["smoke", "full"].contains(&args[5].as_str()) { return Err("invalid comparison matrix parameters".into()); }
            measure(adapter, Spec { workload, smoke: args[5] == "smoke" }, sources, observers).await
        }
        Some(mode @ ("fault-write" | "fault-recover")) if args.len() == 7 => {
            let root = Path::new(&args[1]);
            let token = &args[2];
            let adapter = Adapter::parse(&args[3])?;
            let spec = Spec { workload: Workload::parse(&args[4])?, smoke: false };
            let boundary = Fault::parse(&args[5])?;
            if args[6] != "owned-only" { return Err("fault mode requires explicit owned-only sentinel".into()); }
            if mode == "fault-write" { fault_write(root, token, adapter, &spec, boundary).await }
            else { fault_recover(root, token, adapter, &spec, boundary).await }
        }
        _ => Err("usage: event_blocks_prototype measure <sqlite|files> <text-small|text-large|log|structured|terminal|blob> <1|8 sources> <1|8 observers> <smoke|full>; or fault-{write,recover} <owned-root> <ownership-token> <adapter> <workload> <prepared|transaction|committed> owned-only".into()),
    }
}
