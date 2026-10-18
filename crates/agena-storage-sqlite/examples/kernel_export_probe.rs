//! Owned, fresh-process streaming export probe. No provider or external tool.
//! Example: -- 1000 128 128 1 2 (rows, text MiB, blob MiB, delay ms, slow sinks).

use std::{
    error::Error,
    path::Path,
    pin::Pin,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use agena_domain::{CommandOutputStream, ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, BufReader, BufWriter};

#[path = "support/kernel_resources.rs"]
mod resources;
type AnyResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const MARKER: &str = "owned-kernel-export-probe";

fn url(root: &Path) -> String {
    format!("sqlite://{}?mode=rwc", root.join("kernel.db").display())
}
fn document(text: String) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text }])
}
fn owned(root: &Path, token: &str) -> AnyResult<()> {
    if std::fs::read_to_string(root.join(MARKER))? != token {
        return Err("probe ownership marker mismatch".into());
    }
    Ok(())
}
fn p95(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[(values.len() * 95).div_ceil(100).saturating_sub(1)])
}

fn transaction_percentile_upper_us(buckets: &[u64; 32], percentile: u64) -> Option<u64> {
    let target = (buckets.iter().sum::<u64>() * percentile).div_ceil(100);
    if target == 0 {
        return None;
    }
    let mut seen = 0;
    for (bucket, count) in buckets.iter().enumerate() {
        seen += count;
        if seen >= target {
            return (bucket < 31).then_some(1u64 << bucket);
        }
    }
    None
}

async fn generate(root: &Path, rows: usize, text_mib: usize, blob_mib: usize) -> AnyResult<()> {
    let kernel = open_kernel_runtime(&url(root)).await?;
    let workspace = kernel
        .workspaces
        .create(root.to_string_lossy().into_owned())
        .await?;
    let session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace.id, "export probe".into())
        .await?
        .value
        .id;
    let sent = kernel
        .ports
        .sessions
        .send(RequestId::new(), session, document("export root".into()))
        .await?
        .value;
    let attempt = kernel
        .ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await?
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await?
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "owned-export".into(),
                name: "fixture".into(),
                input: json!({}),
            },
        )
        .await?
        .value;
    let output = round.begin_text(RequestId::new(), false).await?.value;
    let packet = "stream \u{754c}\u{1f642}\r\n".repeat(16_384);
    let total = text_mib * 1024 * 1024;
    let mut appended = 0;
    while appended < total {
        let mut end = (total - appended).min(packet.len());
        while !packet.is_char_boundary(end) {
            end -= 1;
        }
        // The last partial code point is represented by deterministic ASCII.
        if end == 0 {
            output.append(&"z".repeat(total - appended)).await?;
            break;
        }
        output.append(&packet[..end]).await?;
        appended += end;
    }
    output.finish(RequestId::new()).await?;
    let mut input = tokio::io::repeat(0x97).take((blob_mib * 1024 * 1024) as u64);
    let prepared = kernel
        .ports
        .sessions
        .prepare_blob(
            RequestId::new(),
            session,
            (blob_mib * 1024 * 1024) as u64,
            &mut input,
        )
        .await?;
    tool.complete_with_blobs(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"binary":"fixture"}),
        },
        std::slice::from_ref(&prepared),
    )
    .await?;
    round
        .finish(
            RequestId::new(),
            Some(json!({"opaque":[null,true,{"retained":"\u{754c}"}]})),
            true,
        )
        .await?;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await?;
    for index in 1..rows {
        let sent = kernel
            .ports
            .sessions
            .send(
                RequestId::new(),
                session,
                document(format!("{index}: {}", "history \u{754c}\n".repeat(512))),
            )
            .await?
            .value;
        kernel
            .ports
            .sessions
            .cancel(RequestId::new(), session, sent.id)
            .await?;
    }
    std::fs::write(root.join("session"), session.get().to_string())?;
    drop((prepared, output, tool, round, attempt));
    kernel.ports.shutdown().await?;
    drop((
        kernel.maintenance,
        kernel.workspaces,
        kernel.permissions,
        kernel.model_catalog,
    ));
    Ok(())
}

struct SlowFile {
    file: BufWriter<tokio::fs::File>,
    delay: Pin<Box<tokio::time::Sleep>>,
    delay_ms: u64,
    max_write: usize,
}
impl AsyncWrite for SlowFile {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        use std::future::Future;
        let this = self.get_mut();
        if this.delay_ms > 0 && this.delay.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        match Pin::new(&mut this.file).poll_write(cx, bytes) {
            Poll::Ready(Ok(written)) => {
                this.max_write = this.max_write.max(bytes.len());
                this.delay
                    .as_mut()
                    .reset(tokio::time::Instant::now() + Duration::from_millis(this.delay_ms));
                Poll::Ready(Ok(written))
            }
            other => other,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().file).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().file).poll_shutdown(cx)
    }
}

async fn verify(path: &Path, report: &ExportReport) -> AnyResult<Value> {
    let mut reader = BufReader::new(tokio::fs::File::open(path).await?);
    let mut buffer = Vec::new();
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut records = 0u64;
    let mut entries = 0u64;
    let mut largest = 0usize;
    let mut footer = false;
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer).await? == 0 {
            break;
        }
        if footer || buffer.last() != Some(&b'\n') {
            return Err("records after footer or incomplete line".into());
        }
        largest = largest.max(buffer.len());
        let value: Value = serde_json::from_slice(&buffer)?;
        if value["kind"] == "footer" {
            let found: ExportReport = serde_json::from_value(value["data"].clone())?;
            if &found != report
                || bytes != report.bytes
                || records != report.records
                || entries != report.entries
                || <[u8; 32]>::from(hash.clone().finalize()) != report.sha256
            {
                return Err("archive footer/digest/count mismatch".into());
            }
            footer = true;
        } else {
            if value["kind"] == "entry" {
                entries += 1;
            }
            hash.update(&buffer);
            bytes += buffer.len() as u64;
            records += 1;
        }
    }
    if !footer {
        return Err("missing success footer".into());
    }
    Ok(
        json!({"verified":true,"max_line_bytes":largest,"records":records,"entries":entries,"prefix_bytes":bytes}),
    )
}

async fn child(
    root: &Path,
    token: &str,
    delay_ms: u64,
    concurrent_exports: usize,
) -> AnyResult<Value> {
    owned(root, token)?;
    let session =
        SessionId::try_from(std::fs::read_to_string(root.join("session"))?.parse::<i64>()?)?;
    let initial = resources::sample();
    let rss_peak = Arc::new(AtomicU64::new(0));
    let wal_peak = Arc::new(AtomicU64::new(0));
    let sampled = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (rss_peak, wal_peak, done, path) = (
            rss_peak.clone(),
            wal_peak.clone(),
            sampled.clone(),
            root.join("kernel.db-wal"),
        );
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if let Some(usage) = resources::sample() {
                    rss_peak.fetch_max(usage.resident_bytes, Ordering::Relaxed);
                }
                if let Ok(info) = std::fs::metadata(&path) {
                    wal_peak.fetch_max(info.len(), Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let kernel = open_kernel_runtime(&url(root)).await?;
    let snapshot = kernel.ports.reader.history_snapshot(session).await?;
    let root_entries = snapshot.info().entry_count;
    let workspace = snapshot.info().metadata.workspace_id;
    let mut snapshots = vec![snapshot];
    for _ in 1..concurrent_exports {
        let snapshot = kernel.ports.reader.history_snapshot(session).await?;
        if snapshot.info().entry_count != root_entries {
            return Err("export roots do not share the prepared membership".into());
        }
        snapshots.push(snapshot);
    }
    let control_session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace, "concurrent controls".into())
        .await?
        .value
        .id;
    let sent = kernel
        .ports
        .sessions
        .send(
            RequestId::new(),
            control_session,
            document("concurrent streaming producer".into()),
        )
        .await?
        .value;
    let attempt = kernel
        .ports
        .executor
        .claim(RequestId::new(), control_session, sent.id)
        .await?
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await?
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "concurrent".into(),
                name: "fixture".into(),
                input: json!({}),
            },
        )
        .await?
        .value;
    let log = tool.begin_log(RequestId::new()).await?.value;
    let done = Arc::new(AtomicBool::new(false));
    let export_active = Arc::new(
        (0..concurrent_exports)
            .map(|_| AtomicBool::new(false))
            .collect::<Vec<_>>(),
    );
    let control = {
        let (done, commands, reader, export_active) = (
            done.clone(),
            kernel.ports.sessions.clone(),
            kernel.ports.reader.clone(),
            export_active.clone(),
        );
        tokio::spawn(async move {
            let mut tool = tool;
            let mut log = log;
            let mut replies = Vec::new();
            let mut cancels = Vec::new();
            let mut reads = Vec::new();
            let mut appends = Vec::new();
            let mut sends = Vec::new();
            let mut per_export = (0..concurrent_exports)
                .map(|_| {
                    [
                        Vec::<f64>::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ]
                })
                .collect::<Vec<_>>();
            for index in 0..10_000 {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let active_at_start = export_active
                    .iter()
                    .map(|active| active.load(Ordering::Relaxed))
                    .collect::<Vec<_>>();
                let started = Instant::now();
                log.append_at(
                    index,
                    CommandOutputStream::Stdout,
                    &"background \u{754c}\r\n".repeat(256),
                )
                .await?;
                let append_ms = started.elapsed().as_secs_f64() * 1000.0;
                appends.push(append_ms);
                let interaction = tool
                    .request_interaction(
                        RequestId::new(),
                        InteractionRequest::Permission {
                            action_key: format!("probe-{index}"),
                            description: "fixture permission".into(),
                        },
                    )
                    .await?
                    .value;
                let started = Instant::now();
                commands
                    .reply(
                        RequestId::new(),
                        control_session,
                        interaction,
                        InteractionDecision::Permission {
                            reply: agena_domain::PermissionReply {
                                request_id: interaction.get().to_string(),
                                kind: agena_domain::PermissionReplyKind::AllowOnce,
                                reason: None,
                                scope: None,
                            },
                            allowed: true,
                        },
                        None,
                    )
                    .await?;
                let reply_ms = started.elapsed().as_secs_f64() * 1000.0;
                replies.push(reply_ms);
                let started = Instant::now();
                let sent = commands
                    .send(
                        RequestId::new(),
                        session,
                        document(format!("concurrent input {index}")),
                    )
                    .await?
                    .value;
                let send_ms = started.elapsed().as_secs_f64() * 1000.0;
                sends.push(send_ms);
                let started = Instant::now();
                commands.cancel(RequestId::new(), session, sent.id).await?;
                let cancel_ms = started.elapsed().as_secs_f64() * 1000.0;
                cancels.push(cancel_ms);
                let started = Instant::now();
                let page = reader.history(session, None, 20).await?;
                let history_ms = started.elapsed().as_secs_f64() * 1000.0;
                reads.push(history_ms);
                drop(page);
                for (sink, was_active) in active_at_start.into_iter().enumerate() {
                    if was_active && export_active[sink].load(Ordering::Relaxed) {
                        for (samples, value) in per_export[sink]
                            .iter_mut()
                            .zip([append_ms, reply_ms, send_ms, cancel_ms, history_ms])
                        {
                            samples.push(value);
                        }
                    }
                }
                // A tool retains at most 128 interaction facts. Continue the
                // long-running concurrent workload with another real tool,
                // rather than exceeding that durable resource boundary.
                if replies.len().is_multiple_of(128) && !done.load(Ordering::Relaxed) {
                    log.finish(RequestId::new()).await?;
                    tool.complete(
                        RequestId::new(),
                        ToolResult {
                            phase: ToolPhase::Succeeded,
                            value: json!({"interaction_batch":128}),
                        },
                    )
                    .await?;
                    tool = round
                        .begin_tool(
                            RequestId::new(),
                            ToolStart {
                                call_id: format!("concurrent-{}", index + 1),
                                name: "fixture".into(),
                                input: json!({}),
                            },
                        )
                        .await?
                        .value;
                    log = tool.begin_log(RequestId::new()).await?.value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            log.finish(RequestId::new()).await?;
            tool.complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: json!({}),
                },
            )
            .await?;
            round.finish(RequestId::new(), None, true).await?;
            attempt
                .finish(RequestId::new(), ExecutionPhase::Succeeded)
                .await?;
            let per_export = per_export.iter_mut().enumerate().map(|(index, samples)| {
                let [append, reply, send, cancel, history] = samples;
                json!({"sink":index,"samples_wholly_during_export":reply.len(),"append_p95_ms":p95(append),"reply_p95_ms":p95(reply),"send_p95_ms":p95(send),"cancel_p95_ms":p95(cancel),"history_p95_ms":p95(history),"reply_max_ms":reply.iter().copied().reduce(f64::max),"cancel_max_ms":cancel.iter().copied().reduce(f64::max)})
            }).collect::<Vec<_>>();
            Ok::<_, KernelError>(
                json!({"samples":replies.len(),"log_packets":appends.len(),"append_p95_ms":p95(&mut appends),"send_p95_ms":p95(&mut sends),"reply_p95_ms":p95(&mut replies),"cancel_p95_ms":p95(&mut cancels),"history_p95_ms":p95(&mut reads),"reply_max_ms":replies.iter().copied().reduce(f64::max),"cancel_max_ms":cancels.iter().copied().reduce(f64::max),"per_export":per_export}),
            )
        })
    };
    let barrier = Arc::new(tokio::sync::Barrier::new(concurrent_exports + 1));
    let started = Instant::now();
    let mut exports = Vec::with_capacity(concurrent_exports);
    for (index, snapshot) in snapshots.into_iter().enumerate() {
        let path = root.join(format!("archive-{index}.jsonl"));
        let barrier = barrier.clone();
        let active = export_active.clone();
        exports.push(tokio::spawn(async move {
            let mut sink = SlowFile {
                file: BufWriter::with_capacity(256 * 1024, tokio::fs::File::create(&path).await?),
                delay: Box::pin(tokio::time::sleep(Duration::ZERO)),
                delay_ms,
                max_write: 0,
            };
            barrier.wait().await;
            let export_started = Instant::now();
            active[index].store(true, Ordering::Relaxed);
            let result = snapshot.export_jsonl(&mut sink).await;
            let export_finished = Instant::now();
            let elapsed = export_finished.duration_since(export_started).as_secs_f64();
            active[index].store(false, Ordering::Relaxed);
            let report = result?;
            if report.entries != root_entries {
                return Err("concurrent writes entered fixed export membership".into());
            }
            let max_write = sink.max_write;
            drop(sink);
            Ok::<_, Box<dyn Error + Send + Sync>>((
                index,
                path,
                report,
                elapsed,
                max_write,
                export_started.duration_since(started).as_secs_f64() * 1000.0,
                export_finished.duration_since(started).as_secs_f64() * 1000.0,
            ))
        }));
    }
    barrier.wait().await;
    let mut completed = Vec::with_capacity(concurrent_exports);
    for export in exports {
        completed.push(export.await??);
    }
    let elapsed = started.elapsed().as_secs_f64();
    done.store(true, Ordering::Relaxed);
    let control = control.await??;
    let after_export = resources::sample();
    sampled.store(true, Ordering::Relaxed);
    sampler.join().map_err(|_| "resource sampler failed")?;
    let mut verified = Vec::with_capacity(completed.len());
    let overlap_start = completed
        .iter()
        .map(|row| row.5)
        .reduce(f64::max)
        .ok_or("no export ran")?;
    let overlap_end = completed
        .iter()
        .map(|row| row.6)
        .reduce(f64::min)
        .ok_or("no export ran")?;
    let shared_overlap_ms = (overlap_end - overlap_start).max(0.0);
    for (index, path, report, elapsed, max_write, start_ms, finish_ms) in completed {
        let verification = verify(&path, &report).await?;
        verified.push(json!({"sink":index,"report":report,"verification":verification,"elapsed_s":elapsed,"max_sink_write_bytes":max_write,"started_after_group_ms":start_ms,"finished_after_group_ms":finish_ms}));
    }
    let budget = kernel.diagnostics.read_budget();
    let reads = kernel.diagnostics.snapshot();
    let histogram_count = reads.read_transaction_duration_us_log2.iter().sum::<u64>();
    if reads.read_transactions_started == 0
        || reads.read_transactions_started != reads.read_transactions_completed
        || reads.read_transactions_abandoned != 0
        || reads.read_transaction_active_scopes != 0
        || histogram_count != reads.read_transactions_completed
    {
        return Err("export read transaction completion accounting failed".into());
    }
    let transaction_histogram = reads.read_transaction_duration_us_log2.iter().enumerate().map(|(bucket, count)| {
        json!({"upper_bound_us":(bucket < 31).then_some(1u64 << bucket),"count":count,"overflow":bucket == 31})
    }).collect::<Vec<_>>();
    let read_transactions = json!({
        "scope":"explicit archive, pinned history-page and blob-range read transactions; implicit single-statement reads excluded",
        "timing":"acknowledged BEGIN to acknowledged ROLLBACK; pool acquisition/BEGIN wait excluded",
        "started":reads.read_transactions_started,
        "completed":reads.read_transactions_completed,
        "abandoned_end_unobserved":reads.read_transactions_abandoned,
        "active_measurement_scopes":reads.read_transaction_active_scopes,
        "peak_active_measurement_scopes":reads.read_transaction_peak_active_scopes,
        "completed_total_ns":reads.read_transaction_total_completed_ns,
        "completed_mean_ns":reads.read_transaction_total_completed_ns / reads.read_transactions_completed,
        "completed_max_ns":reads.read_transaction_max_completed_ns,
        "completed_p50_upper_us":transaction_percentile_upper_us(&reads.read_transaction_duration_us_log2, 50),
        "completed_p95_upper_us":transaction_percentile_upper_us(&reads.read_transaction_duration_us_log2, 95),
        "completed_duration_histogram":transaction_histogram,
    });
    kernel.maintenance.collect_history_snapshots().await?;
    let schema_bytes = std::fs::metadata(root.join("kernel.db"))?.len();
    kernel.ports.shutdown().await?;
    Ok(
        json!({"mode":"fresh_process_sqlite_full","scope":"isolated_kernel_concurrent_slow_exports","sink_delay_ms":delay_ms,"concurrent_exports":concurrent_exports,"all_exports_verified":true,"shared_export_overlap_ms":shared_overlap_ms,"exports":verified,"elapsed_s":elapsed,"control":control,"initial_usage":initial,"after_export_usage":after_export,"rss_peak_sampled_bytes":(rss_peak.load(Ordering::Relaxed)>0).then(||rss_peak.load(Ordering::Relaxed)),"wal_peak_sampled_bytes":wal_peak.load(Ordering::Relaxed),"sample_interval_ms":10,"read_budget":budget,"read_transactions":read_transactions,"database_bytes":schema_bytes,"provider_enabled":false,"os_cache_evicted":false}),
    )
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> AnyResult<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|v| v == "--child") {
        println!(
            "{}",
            child(
                Path::new(&args[1]),
                &args[2],
                args[3].parse()?,
                args[4].parse()?
            )
            .await?
        );
    } else {
        if args.len() != 5 {
            return Err(
                "expected: history_rows text_mib blob_mib sink_delay_ms concurrent_exports".into(),
            );
        }
        let rows = args[0].parse::<usize>()?;
        let text = args[1].parse::<usize>()?;
        let blobs = args[2].parse::<usize>()?;
        let concurrent_exports = args[4].parse::<usize>()?;
        if rows == 0
            || rows > 100_000
            || text > 1024
            || blobs > 1024
            || !(1..=2).contains(&concurrent_exports)
        {
            return Err("probe bounds exceeded".into());
        }
        let root = tempfile::tempdir()?;
        let token = RequestId::new().to_string();
        std::fs::write(root.path().join(MARKER), &token)?;
        generate(root.path(), rows, text, blobs).await?;
        let output = Command::new(std::env::current_exe()?)
            .arg("--child")
            .arg(root.path())
            .arg(&token)
            .arg(&args[3])
            .arg(concurrent_exports.to_string())
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "export child failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let mut result: Value = serde_json::from_slice(&output.stdout)?;
        result["fixture"] =
            json!({"history_user_rows":rows,"text_mib":text,"blob_mib":blobs,"owned_tempdir":true});
        println!("{result}");
    }
    Ok(())
}
