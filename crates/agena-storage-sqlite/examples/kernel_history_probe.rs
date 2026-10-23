//! Isolated B03/B04 release probe. A generator process creates valid completed
//! history, then exits before fresh reader processes measure the actual kernel.
//! Parent mode owns the temporary directory; child paths require its marker.

use std::{
    error::Error,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use sea_orm::sqlx;
use serde_json::{Value, json};

#[path = "support/kernel_resources.rs"]
mod resources;

type AnyResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const MARKER: &str = "owned-kernel-history-probe";
const COLD_TEXT_BYTES: usize = 240 * 1024;
const REOPENS: usize = 5;
const WARM_READS: usize = 512;

fn text(bytes: usize) -> String {
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_-";
    let mut state = 0x2026101842303344u64;
    (0..bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            alphabet[(state & 63) as usize] as char
        })
        .collect()
}
fn url(root: &Path) -> String {
    format!("sqlite://{}?mode=rwc", root.join("kernel.db").display())
}
fn check_owned(root: &Path, token: &str) -> AnyResult<()> {
    if std::fs::read_to_string(root.join(MARKER))? != token {
        return Err("missing probe ownership marker".into());
    }
    Ok(())
}
fn percentile(values: &mut [f64], percent: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    values[(values.len() * percent).div_ceil(100).saturating_sub(1)]
}

async fn generate(root: &Path, token: &str, recent: usize, cold_mib: usize) -> AnyResult<Value> {
    check_owned(root, token)?;
    if root.join("kernel.db").exists() {
        return Err("generator requires a fresh owned database".into());
    }
    let kernel = open_kernel_runtime(&url(root)).await?;
    let workspace = kernel
        .workspaces
        .create(root.to_string_lossy().into_owned())
        .await?;
    let session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace.id, "read probe".into())
        .await?
        .value
        .id;
    kernel.ports.clone().shutdown().await?;
    drop(kernel);
    // The kernel has closed. This isolated generator alone now owns its
    // marker-protected fixture; no raw connection is added to the kernel API.
    let fixture_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(root.join("kernel.db"))
                .foreign_keys(true)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Full)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .pragma("cache_size", "-8192")
                .pragma("mmap_size", "0"),
        )
        .await?;
    let pool = &fixture_pool;
    let sqlite_version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(pool)
        .await?;
    let empty_session_pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await?;
    let initial_page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(pool)
        .await?;
    let cold_entries = (cold_mib * 1024 * 1024).div_ceil(COLD_TEXT_BYTES);
    let total = recent + cold_entries;
    let small = text(512);
    let large = text(COLD_TEXT_BYTES);
    let started = Instant::now();
    // The fixture builder is infrastructure, not a second business writer.
    // Every synthetic entry has a valid user boundary, execution, succeeded
    // attempt and immutable membership. It does not create queued side effects.
    for from in (1..=total).step_by(128) {
        let end = (from + 128).min(total + 1).min(if from <= cold_entries {
            cold_entries + 1
        } else {
            total + 1
        });
        // Do not cross the cold/recent boundary in a chunk: recent bodies stay
        // the same size even when there are GiB of earlier valid cold bodies.
        let mut position = from;
        while position < ((from + 128).min(total + 1)) {
            let boundary = if position <= cold_entries {
                end
            } else {
                (from + 128).min(total + 1)
            };
            let rows = (position..boundary)
                .map(|id| json!({"id":id,"execution":format!("20261018-0000-4000-8000-{id:012x}")}))
                .collect::<Vec<_>>();
            let identities = serde_json::to_string(&rows)?;
            let body = if position <= cold_entries {
                &large
            } else {
                &small
            };
            let mut tx = pool.begin().await?;
            sqlx::query("INSERT INTO agena_executions (id,session_id,message_id,current_generation,created_at_ms) SELECT json_extract(value,'$.execution'),?,json_extract(value,'$.id'),1,0 FROM json_each(?)")
                .bind(session.get()).bind(&identities).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO agena_attempts (execution_id,generation,session_id,phase,finished_at_ms) SELECT json_extract(value,'$.execution'),1,?,'succeeded',0 FROM json_each(?)")
                .bind(session.get()).bind(&identities).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO agena_entries (id,origin_session_id,execution_id,generation,kind,body_json,created_at_ms) SELECT json_extract(value,'$.id'),?,json_extract(value,'$.execution'),1,'user',json_array(json_object('type','text','text',? || ' #' || json_extract(value,'$.id'))),0 FROM json_each(?)")
                .bind(session.get()).bind(body).bind(&identities).execute(&mut *tx).await?;
            tx.commit().await?;
            position = boundary;
        }
        let segment = (from - 1) / 128 + 1;
        let mut tx = pool.begin().await?;
        sqlx::query("INSERT INTO agena_history_segments (id) VALUES (?)")
            .bind(segment as i64)
            .execute(&mut *tx)
            .await?;
        let identities =
            serde_json::to_string(&(from..(from + 128).min(total + 1)).collect::<Vec<_>>())?;
        sqlx::query("INSERT INTO agena_segment_entries (segment_id,ordinal,entry_id) SELECT ?,CAST(key AS INTEGER),value FROM json_each(?)")
            .bind(segment as i64).bind(identities).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO agena_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal) VALUES (?,?,?,0,?)")
            .bind(session.get()).bind(segment as i64-1).bind(segment as i64).bind((total+1-from).min(128) as i64).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    sqlx::query("UPDATE agena_sessions SET membership_generation=2 WHERE id=?")
        .bind(session.get())
        .execute(pool)
        .await?;
    sqlx::query("UPDATE agena_session_statistics SET entry_count=?,user_message_count=?,last_message_at_ms=0 WHERE session_id=?")
        .bind(total as i64).bind(total as i64).bind(session.get()).execute(pool).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(pool)
        .await?;
    if integrity != "ok"
        || !sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await?
            .is_empty()
    {
        return Err("generated fixture is not relationally valid".into());
    }
    let cold_body_bytes:i64=sqlx::query_scalar("SELECT COALESCE(SUM(body_bytes),0) FROM agena_entries INDEXED BY idx_agena_entry_bytes WHERE id<=?")
        .bind(cold_entries as i64).fetch_one(pool).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_entries")
        .fetch_one(pool)
        .await?;
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(pool)
        .await?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(pool)
        .await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await?;
    fixture_pool.close().await;
    if count != total as i64 || cold_body_bytes < (cold_mib * 1024 * 1024) as i64 {
        return Err("fixture data target was not reached".into());
    }
    Ok(
        json!({"recent_entries":recent,"cold_entries":cold_entries,"entries":total,"cold_body_bytes":cold_body_bytes,"database_page_bytes":pages*page_size,"sqlite_version":sqlite_version,"schema_with_empty_session_page_bytes":empty_session_pages*initial_page_size,"generation_ms":started.elapsed().as_secs_f64()*1000.0,"integrity":"quick_check + foreign_key_check passed"}),
    )
}

async fn measure(root: &Path, token: &str) -> AnyResult<Value> {
    check_owned(root, token)?;
    let peak = Arc::new(AtomicU64::new(0));
    let sampling = Arc::new(AtomicBool::new(true));
    let sampler = {
        let peak = peak.clone();
        let sampling = sampling.clone();
        std::thread::spawn(move || {
            while sampling.load(Ordering::Relaxed) {
                if let Some(usage) = resources::sample() {
                    peak.fetch_max(usage.resident_bytes, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let initial = resources::sample();
    let mut reopens = Vec::new();
    for reopen in 0..REOPENS {
        let before = resources::sample();
        let now = Instant::now();
        let SqliteKernel {
            database,
            ports,
            diagnostics,
            maintenance,
            workspaces,
            permissions,
            model_catalog,
            ..
        } = open_kernel_runtime(&url(root)).await?;
        let open_ms = now.elapsed().as_secs_f64() * 1000.0;
        drop((database, workspaces, permissions, model_catalog));
        let session = SessionId::try_from(1)?;
        let cold_start = Instant::now();
        let page = ports.reader.history(session, None, 100).await?;
        let cold_page_ms = cold_start.elapsed().as_secs_f64() * 1000.0;
        if page.entries.len() != 100
            || !page.has_more
            || !page.entries.windows(2).all(|p| p[0].id < p[1].id)
        {
            return Err("recent page identity/order mismatch".into());
        }
        for entry in &page.entries {
            match &entry.payload {
                EntryPayload::User { document } if !document.is_empty() => {}
                _ => {
                    return Err(
                        "fixture recent entry was not decoded through the real kernel".into(),
                    );
                }
            }
        }
        let cold_metrics = diagnostics.snapshot();
        let mut warm_ms = Vec::new();
        let warm_start = diagnostics.snapshot();
        for index in 0..WARM_READS {
            let now = Instant::now();
            let hit = ports.reader.history(session, None, 100).await?;
            warm_ms.push(now.elapsed().as_secs_f64() * 1000.0);
            if !page.ptr_eq(&hit) {
                return Err("warm page copied or missed".into());
            }
            if index.is_multiple_of(32) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        let warm_end = diagnostics.snapshot();
        let warm_queries = warm_end.history_queries - warm_start.history_queries;
        let warm_rows = warm_end.history_entry_rows - warm_start.history_entry_rows;
        let warm_body_bytes =
            warm_end.history_entry_body_bytes - warm_start.history_entry_body_bytes;
        if warm_queries != 0 || warm_rows != 0 || warm_body_bytes != 0 {
            return Err("valid warm history executed SQL".into());
        }
        let warm_usage = resources::sample();
        let now = Instant::now();
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some(format!("reopen {reopen}")),
                    ..Default::default()
                },
            )
            .await?;
        let metadata_update_ms = now.elapsed().as_secs_f64() * 1000.0;
        let update_start = diagnostics.snapshot();
        let updated = ports.reader.history(session, None, 100).await?;
        if updated.metadata.title != format!("reopen {reopen}") {
            return Err("cache retained stale metadata after a receipt".into());
        }
        let update_end = diagnostics.snapshot();
        let now = Instant::now();
        let sent = ports
            .sessions
            .send(
                RequestId::new(),
                session,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "probe cancel".into(),
                }]),
            )
            .await?;
        let send_ms = now.elapsed().as_secs_f64() * 1000.0;
        let now = Instant::now();
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.value.id)
            .await?;
        let cancel_ms = now.elapsed().as_secs_f64() * 1000.0;
        let cache = diagnostics.read_cache();
        let budget = diagnostics.read_budget();
        drop((page, updated, maintenance));
        ports.shutdown().await?;
        let after_close = resources::sample();
        if diagnostics.read_budget().bytes != 0 {
            return Err("kernel close retained a read allocation".into());
        }
        reopens.push(json!({"reopen":reopen,"open_ms":open_ms,"cold_page_ms":cold_page_ms,"cold_metrics":cold_metrics,"warm_reads":WARM_READS,"warm_queries":warm_queries,"warm_entry_rows":warm_rows,"warm_body_bytes":warm_body_bytes,"warm_p50_ms":percentile(&mut warm_ms,50),"warm_p95_ms":percentile(&mut warm_ms,95),"metadata_update_ms":metadata_update_ms,"send_ms":send_ms,"cancel_ms":cancel_ms,"update_page_rows":update_end.history_entry_rows-update_start.history_entry_rows,"update_page_body_bytes":update_end.history_entry_body_bytes-update_start.history_entry_body_bytes,"cache":cache,"read_budget":budget,"usage_before":before,"usage_warm":warm_usage,"usage_after_close":after_close}));
    }
    sampling.store(false, Ordering::Relaxed);
    sampler.join().map_err(|_| "RSS sampler panicked")?;
    Ok(
        json!({"initial_usage":initial,"final_usage":resources::sample(),"rss_peak_sampled_bytes":peak.load(Ordering::Relaxed),"reopens":reopens,"provider_enabled":false,"rss_sample_interval_ms":10}),
    )
}

fn child(executable: &Path, args: &[&str]) -> AnyResult<Value> {
    let output = Command::new(executable).args(args).output()?;
    if !output.status.success() {
        return Err(format!(
            "probe child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> AnyResult<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let result = match args.get(1).map(String::as_str) {
        Some("--generate") => {
            generate(
                Path::new(&args[2]),
                &args[3],
                args[4].parse()?,
                args[5].parse()?,
            )
            .await?
        }
        Some("--measure") => measure(Path::new(&args[2]), &args[3]).await?,
        _ => {
            let recent: usize = args.get(1).map_or(Ok(1000), |s| s.parse())?;
            let cold_mib: usize = args.get(2).map_or(Ok(0), |s| s.parse())?;
            if ![1000, 10000, 100000].contains(&recent) || ![0, 64, 1024].contains(&cold_mib) {
                return Err(
                    "usage: kernel_history_probe [1000|10000|100000 entries] [0|64|1024 cold MiB]"
                        .into(),
                );
            }
            let executable = std::env::current_exe()?;
            let target = executable
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .ok_or("example is not under a Cargo target directory")?;
            let root = tempfile::Builder::new()
                .prefix("agena-kernel-read-probe-")
                .tempdir_in(target)?;
            let token = RequestId::new().to_string();
            std::fs::write(root.path().join(MARKER), &token)?;
            let path = root.path().to_str().ok_or("probe temp path is not UTF-8")?;
            let fixture = child(
                &executable,
                &[
                    "--generate",
                    path,
                    &token,
                    &recent.to_string(),
                    &cold_mib.to_string(),
                ],
            )?;
            let mut runs = Vec::new();
            for repeat in 1..=3 {
                let mut run = child(&executable, &["--measure", path, &token])?;
                run["repeat"] = repeat.into();
                runs.push(run);
            }
            json!({"recent_entries":recent,"cold_mib":cold_mib,"fixture":fixture,"runs":runs,"definition":"cold = new kernel/process with empty view cache; OS file cache is not evicted; generator process has exited; reader makes five reopens; 512 known-valid warm reads per reopen"})
        }
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
