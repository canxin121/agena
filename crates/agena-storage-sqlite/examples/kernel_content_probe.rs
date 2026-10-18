//! B01/B02 measurements through the actual new kernel's public capabilities.
//! All data lives in an owned temporary directory. This measures kernel range
//! readers, not a provider/network/client UI. See the P4 evidence for limits.

use std::{
    collections::VecDeque,
    error::Error,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use agena_domain::{CommandOutputStream, ComposerDocument, ComposerNode, ContentId};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;
use sea_orm::sqlx::{
    self,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use serde_json::json;
use sha2::{Digest, Sha256};

#[path = "support/kernel_resources.rs"]
mod resources;

type AnyResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const SOURCE_BYTES: usize = 8192 * 17;
const BUCKETS: usize = 10001;

#[derive(Clone)]
enum ProbeOutput {
    Text(TextOutput),
    Log(LogOutput),
}
impl ProbeOutput {
    fn id(&self) -> ContentId {
        match self {
            Self::Text(output) => output.id(),
            Self::Log(output) => output.id(),
        }
    }
    async fn append(
        &self,
        source: usize,
        index: usize,
        text: &str,
    ) -> agena_storage::kernel::Result<u64> {
        match self {
            Self::Text(output) => Ok(output.append(text).await?.offset),
            Self::Log(output) => Ok(output
                .append_at(timestamp(source, index), channel(index), text)
                .await?
                .sequence),
        }
    }
}
fn channel(index: usize) -> CommandOutputStream {
    if index.is_multiple_of(2) {
        CommandOutputStream::Stdout
    } else {
        CommandOutputStream::Stderr
    }
}
fn timestamp(source: usize, index: usize) -> i64 {
    1_792_281_600_000 + (source * 10_000 + index) as i64
}

struct Histogram {
    counts: Vec<u64>,
    count: u64,
    total_us: u128,
    max_us: u64,
}
impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: vec![0; BUCKETS],
            count: 0,
            total_us: 0,
            max_us: 0,
        }
    }
}
impl Histogram {
    fn record(&mut self, time: Duration) {
        let us = time.as_micros().min(u64::MAX as u128) as u64;
        self.counts[(us / 10).min((BUCKETS - 1) as u64) as usize] += 1;
        self.count += 1;
        self.total_us += u128::from(us);
        self.max_us = self.max_us.max(us);
    }
    fn percentile(&self, percent: u64) -> Option<f64> {
        if self.count == 0 {
            return None;
        }
        let rank = (self.count * percent).div_ceil(100);
        let mut count = 0;
        for (index, number) in self.counts.iter().enumerate() {
            count += number;
            if count >= rank {
                return (index < BUCKETS - 1).then_some((index + 1) as f64 / 100.0);
            }
        }
        None
    }
    fn json(&self) -> serde_json::Value {
        json!({"count":self.count,"p50_upper_ms":self.percentile(50),"p95_upper_ms":self.percentile(95),"mean_ms":(self.count>0).then(||self.total_us as f64/self.count as f64/1000.0),"max_ms":self.max_us as f64/1000.0,"overflow_ge_100ms":self.counts[BUCKETS-1],"bucket_width_us":10})
    }
}

type Markers = Arc<Mutex<VecDeque<(u64, Instant)>>>;
fn mark(markers: &Markers, end: u64, now: Instant) {
    let mut markers = markers.lock().unwrap();
    markers.push_back((end, now));
    while markers.len() > 256 {
        markers.pop_front();
    }
}
fn elapsed(markers: &Markers, end: u64) -> Option<Duration> {
    markers
        .lock()
        .unwrap()
        .iter()
        .rev()
        // A physical block/page boundary may split an append. Measure the
        // newest fully included append whose bounded marker is still present.
        // Missing/evicted markers produce no sample, never a zero latency.
        .find(|(offset, _)| *offset <= end)
        .map(|(_, time)| time.elapsed())
}
fn body(source: usize) -> String {
    let mut state = 0x20261018534b3031u64 ^ (source as u64).wrapping_mul(0x9e3779b97f4a7c15);
    let mut bytes = Vec::with_capacity(SOURCE_BYTES);
    for _ in 0..SOURCE_BYTES {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push(33 + (state % 94) as u8);
    }
    String::from_utf8(bytes).unwrap()
}
async fn page_bytes(pool: &sqlx::SqlitePool) -> AnyResult<i64> {
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await?;
    let page: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(pool)
        .await?;
    Ok(pages * page)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> AnyResult<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let sources: usize = args.get(1).map_or(Ok(1), |v| v.parse())?;
    let observers: usize = args.get(2).map_or(Ok(1), |v| v.parse())?;
    let slicing = args.get(3).map_or("small", String::as_str);
    let kind = args.get(4).map_or("text", String::as_str);
    let is_log = kind == "log";
    let packet_bytes = if slicing == "small" {
        17
    } else {
        SOURCE_BYTES / 4
    };
    let packets_per_source = SOURCE_BYTES / packet_bytes;
    if ![1, 8, 32].contains(&sources)
        || ![1, 8].contains(&observers)
        || !["small", "large"].contains(&slicing)
        || !["text", "log"].contains(&kind)
        || args.len() > 5
    {
        return Err(
            "usage: kernel_content_probe [1|8|32 sources] [1|8 observers] [small|large] [text|log]"
                .into(),
        );
    }
    let root = tempfile::tempdir()?;
    let path = root.path().join("kernel.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let kernel = open_kernel_runtime(&url).await?;
    // Read-only measurement connection to this probe's own DB. It grants no
    // raw connection to product callers and adds no instrumenting SQL writes.
    let inspect = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::from_str(&url)?.read_only(true))
        .await?;
    let empty_bytes = page_bytes(&inspect).await?;
    let sqlite_version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&inspect)
        .await?;
    let workspace = kernel
        .workspaces
        .create(root.path().to_string_lossy().into_owned())
        .await?;
    let ports = &kernel.ports;
    let session = ports
        .sessions
        .create(RequestId::new(), workspace.id, "content probe".into())
        .await?
        .value
        .id;
    let sent = ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "isolated content probe".into(),
            }]),
        )
        .await?
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await?
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await?
        .value;
    let accepted = Arc::new(Mutex::new(Histogram::default()));
    let visible = Arc::new(Mutex::new(Histogram::default()));
    let durable = Arc::new(Mutex::new(Histogram::default()));
    let reader_busy = Arc::new(AtomicU64::new(0));
    let producer_busy = Arc::new(AtomicU64::new(0));
    let tool = if is_log {
        Some(
            round
                .begin_tool(
                    RequestId::new(),
                    ToolStart {
                        call_id: "log-matrix".into(),
                        name: "owned-log-producer".into(),
                        input: json!({"sources":sources}),
                    },
                )
                .await?
                .value,
        )
    } else {
        None
    };
    let mut outputs = Vec::new();
    let mut marker_sets = Vec::new();
    let mut readers = Vec::new();
    for source in 0..sources {
        let output = if let Some(tool) = &tool {
            ProbeOutput::Log(tool.begin_log(RequestId::new()).await?.value)
        } else {
            ProbeOutput::Text(round.begin_text(RequestId::new(), false).await?.value)
        };
        let markers: Markers = Arc::new(Mutex::new(VecDeque::new()));
        let expected: [u8; 32] = Sha256::digest(body(source).as_bytes()).into();
        for _ in 0..observers {
            let mut watch = ports.reader.watch_content(session, output.id()).await?;
            let reader = ports.reader.clone();
            let content = output.id();
            let markers = markers.clone();
            let visible = visible.clone();
            let durable = durable.clone();
            let busy = reader_busy.clone();
            readers.push(tokio::spawn(async move {
                let mut text_cursor = None;
                let mut event_cursor = None;
                let mut position = 0u64;
                let mut bytes = 0;
                let mut events = 0usize;
                let mut checksum = Sha256::new();
                let mut committed = 0;
                loop {
                    let head = watch.head()?;
                    if head.committed > committed {
                        committed = head.committed;
                        if let Some(time) = elapsed(&markers, committed) {
                            durable.lock().unwrap().record(time);
                        }
                    }
                    if position < head.accepted {
                        if is_log {
                            let page =
                                match reader.event_page(session, content, event_cursor, 32).await {
                                    Ok(page) => page,
                                    Err(KernelError::Busy) => {
                                        busy.fetch_add(1, Ordering::Relaxed);
                                        tokio::task::yield_now().await;
                                        continue;
                                    }
                                    Err(error) => return Err(error.into()),
                                };
                            for record in &page.records {
                                let EventPayload::Log {
                                    channel: observed,
                                    text,
                                } = &record.payload
                                else {
                                    return Err("log matrix exposed a non-log event".into());
                                };
                                if record.sequence != events as u64 + 1
                                    || record.timestamp_ms != timestamp(source, events)
                                    || *observed != channel(events)
                                    || text.len() != packet_bytes
                                {
                                    return Err(
                                        "log packet order/channel/time/boundary mismatch".into()
                                    );
                                }
                                checksum.update(text.as_bytes());
                                bytes += text.len() as u64;
                                events += 1;
                            }
                            if page.next.sequence <= position {
                                return Err("log range made no progress".into());
                            }
                            position = page.next.sequence;
                            event_cursor = Some(page.next);
                        } else {
                            let page = match reader
                                .text_range(session, content, text_cursor, 256 * 1024)
                                .await
                            {
                                Ok(page) => page,
                                Err(KernelError::Busy) => {
                                    busy.fetch_add(1, Ordering::Relaxed);
                                    tokio::task::yield_now().await;
                                    continue;
                                }
                                Err(error) => return Err(error.into()),
                            };
                            checksum.update(&page.bytes);
                            bytes = page.next.offset;
                            position = bytes;
                            text_cursor = Some(page.next);
                        }
                        if let Some(time) = elapsed(&markers, position) {
                            visible.lock().unwrap().record(time);
                        }
                        continue;
                    }
                    if head.state != CaptureState::Open {
                        break;
                    }
                    watch.changed().await?;
                }
                if bytes != SOURCE_BYTES as u64
                    || <[u8; 32]>::from(checksum.finalize()) != expected
                    || is_log && events != packets_per_source
                {
                    return Err("observer byte count/checksum mismatch".into());
                }
                Ok::<(), Box<dyn Error + Send + Sync>>(())
            }));
        }
        outputs.push(output);
        marker_sets.push(markers);
    }
    let base_bytes = page_bytes(&inspect).await?;
    let before = kernel.diagnostics.snapshot();
    let usage_before = resources::sample();
    let peak_rss = Arc::new(AtomicU64::new(usage_before.map_or(0, |u| u.resident_bytes)));
    let sampling = Arc::new(AtomicBool::new(true));
    let peak = peak_rss.clone();
    let active = sampling.clone();
    let sampler = tokio::spawn(async move {
        while active.load(Ordering::Relaxed) {
            if let Some(usage) = resources::sample() {
                peak.fetch_max(usage.resident_bytes, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let started = Instant::now();
    let mut producers = Vec::new();
    for (source, (output, markers)) in outputs.iter().cloned().zip(marker_sets).enumerate() {
        let accepted = accepted.clone();
        let busy = producer_busy.clone();
        producers.push(tokio::spawn(async move {
            let text = body(source);
            let mut offset = 0;
            for (index, chunk) in text.as_bytes().chunks(packet_bytes).enumerate() {
                let now = Instant::now();
                offset += chunk.len() as u64;
                let expected_position = if is_log { index as u64 + 1 } else { offset };
                mark(&markers, expected_position, now);
                let cursor = loop {
                    match output
                        .append(source, index, std::str::from_utf8(chunk)?)
                        .await
                    {
                        Ok(cursor) => break cursor,
                        Err(KernelError::Busy) if now.elapsed() < Duration::from_secs(30) => {
                            busy.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Err(error) => return Err(error.into()),
                    }
                };
                accepted.lock().unwrap().record(now.elapsed());
                if cursor != expected_position {
                    return Err("accepted cursor mismatch".into());
                }
            }
            Ok::<(), Box<dyn Error + Send + Sync>>(())
        }));
    }
    for producer in producers {
        producer.await??;
    }
    let finish = Instant::now();
    if let Some(tool) = &tool {
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({"capture":"matrix completed"}),
            },
        )
        .await?;
    } else {
        round.finish(RequestId::new(), None, true).await?;
    }
    let finish_ms = finish.elapsed().as_secs_f64() * 1000.0;
    for reader in readers {
        reader.await??;
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = kernel.diagnostics.snapshot();
    let blocks: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_content_blocks")
        .fetch_one(&inspect)
        .await?;
    let stored_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(sum(length(body)),0) FROM agena_content_blocks")
            .fetch_one(&inspect)
            .await?;
    let final_page_bytes = page_bytes(&inspect).await?;
    let appends = sources as u64 * packets_per_source as u64;
    let expected_stored_bytes = SOURCE_BYTES as i64 * sources as i64
        + if is_log {
            appends as i64 * 13 + blocks * 4
        } else {
            0
        };
    let measured_blocks = if is_log {
        after.event_blocks - before.event_blocks
    } else {
        after.text_blocks - before.text_blocks
    };
    let accepted_appends = if is_log {
        after.accepted_events - before.accepted_events
    } else {
        after.accepted_text_appends - before.accepted_text_appends
    };
    if stored_bytes != expected_stored_bytes
        || blocks as u64 != measured_blocks
        || after.command_results - before.command_results != 1
        || after.command_transactions - before.command_transactions != 1
        || !is_log
            && after.accepted_text_bytes - before.accepted_text_bytes
                != SOURCE_BYTES as u64 * sources as u64
        || accepted_appends != appends
    {
        return Err("diagnostic counters or durable representation mismatch".into());
    }
    let wal_bytes = std::fs::metadata(format!("{}-wal", path.display())).map_or(0, |m| m.len());
    inspect.close().await;
    drop(outputs);
    drop(tool);
    drop(round);
    drop(attempt);
    kernel.ports.shutdown().await?;
    sampling.store(false, Ordering::Relaxed);
    sampler.await?;
    let usage_after = resources::sample();
    println!(
        "{}",
        json!({"probe":"new_kernel_B01_B02","sqlite_version":sqlite_version,"kind":kind,"sources":sources,"observers_per_source":observers,"slicing":slicing,"payload_bytes_per_source":SOURCE_BYTES,"packets_per_source":packets_per_source,"all_observers_exact":true,"all_log_order_channels_timestamps_exact":is_log.then_some(true),"source_sha256":(0..sources).map(|source|hex::encode(Sha256::digest(body(source).as_bytes()))).collect::<Vec<_>>(),"elapsed_ms":elapsed_ms,"terminal_finish_ms":finish_ms,"terminal_command":if is_log {"complete_tool"}else{"finish_round"},"round_finish_ms":(!is_log).then_some(finish_ms),"append_call_to_accepted":accepted.lock().unwrap().json(),"append_call_to_range_read":visible.lock().unwrap().json(),"append_call_to_committed_observation":durable.lock().unwrap().json(),"reader_busy_retries":reader_busy.load(Ordering::Relaxed),"producer_busy_retries":producer_busy.load(Ordering::Relaxed),"empty_database_page_bytes":empty_bytes,"before_content_page_bytes":base_bytes,"final_page_bytes":final_page_bytes,"content_increment_page_bytes":final_page_bytes-base_bytes,"closed_database_file_bytes":std::fs::metadata(&path)?.len(),"wal_file_bytes_at_final_observation":wal_bytes,"blocks":blocks,"stored_body_bytes":stored_bytes,"expected_stored_body_bytes":expected_stored_bytes,"command_transactions_during_content":after.command_transactions-before.command_transactions,"command_results_during_content":after.command_results-before.command_results,"checkpoint_transactions":after.checkpoint_transactions-before.checkpoint_transactions,"accepted_appends":accepted_appends,"os_usage_before_content":usage_before,"os_usage_after_close":usage_after,"rss_peak_sampled_bytes":(peak_rss.load(Ordering::Relaxed)>0).then(||peak_rss.load(Ordering::Relaxed)),"os_disk_written_bytes_content_through_close":usage_before.zip(usage_after).map(|(a,b)|b.disk_written_bytes.saturating_sub(a.disk_written_bytes)),"os_disk_read_bytes_content_through_close":usage_before.zip(usage_after).map(|(a,b)|b.disk_read_bytes.saturating_sub(a.disk_read_bytes)),"notes":"FULL/WAL; actual new kernel; text native positions are bytes and log native positions are ordered events; Log preserves 13-byte channel/time/length frames plus 4 bytes/block; histogram upper bounds use 10 us bins, null p95 means >=100 ms; range readers are not UI; observer metrics are coalesced samples and include admission; OS writes are attributed disk I/O, not total device amplification; RSS includes harness and SQLite; WAL length is not cumulative writes"})
    );
    Ok(())
}
