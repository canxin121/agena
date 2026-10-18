//! B06: valid immutable history, wide/deep forks, active shared content and GC.

use super::*;

pub(super) async fn generate(root: &Path, token: &str, entries: usize) -> AnyResult<Value> {
    check_owned(root, token)?;
    if entries == 0 || root.join("kernel.db").exists() {
        return Err("B06 fixture requires positive entries and a fresh owned database".into());
    }
    let kernel = open_kernel_runtime(&database_url(root)).await?;
    let workspace = kernel
        .workspaces
        .create(root.to_string_lossy().into_owned())
        .await?;
    let session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace.id, "B06 shared history".into())
        .await?
        .value
        .id;
    kernel.ports.clone().shutdown().await?;
    drop(kernel);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str(&database_url(root))?
                .foreign_keys(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .synchronous(sqlx::sqlite::SqliteSynchronous::Full)
                .pragma("cache_size", "-8192")
                .pragma("mmap_size", "0"),
        )
        .await?;
    let started = Instant::now();
    let body = ascii(512, 0xb0620261018);
    for from in (1..=entries).step_by(128) {
        let end = (from + 128).min(entries + 1);
        let identities = serde_json::to_string(
            &(from..end)
                .map(|id| json!({"id":id,"execution":format!("20261018-0000-4000-8000-{id:012x}")}))
                .collect::<Vec<_>>(),
        )?;
        let mut tx = pool.begin().await?;
        sqlx::query("INSERT INTO agena_executions (id,session_id,message_id,current_generation,created_at_ms) SELECT json_extract(value,'$.execution'),?,json_extract(value,'$.id'),1,0 FROM json_each(?)")
            .bind(session.get()).bind(&identities).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO agena_attempts (execution_id,generation,session_id,phase,finished_at_ms) SELECT json_extract(value,'$.execution'),1,?,'succeeded',0 FROM json_each(?)")
            .bind(session.get()).bind(&identities).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO agena_entries (id,origin_session_id,execution_id,generation,kind,body_json,created_at_ms) SELECT json_extract(value,'$.id'),?,json_extract(value,'$.execution'),1,'user',json_array(json_object('type','text','text',? || ' #' || json_extract(value,'$.id'))),0 FROM json_each(?)")
            .bind(session.get()).bind(&body).bind(&identities).execute(&mut *tx).await?;
        let segment: i64 =
            sqlx::query_scalar("INSERT INTO agena_history_segments DEFAULT VALUES RETURNING id")
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("INSERT INTO agena_segment_entries (segment_id,ordinal,entry_id) SELECT ?,CAST(key AS INTEGER),json_extract(value,'$.id') FROM json_each(?)")
            .bind(segment).bind(&identities).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO agena_session_segments (session_id,sequence,segment_id,first_ordinal,end_ordinal) VALUES (?,?,?,0,?)")
            .bind(session.get()).bind(((from-1)/128) as i64).bind(segment).bind((end-from) as i64).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    sqlx::query("UPDATE agena_sessions SET membership_generation=2 WHERE id=?")
        .bind(session.get())
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE agena_session_statistics SET entry_count=?,user_message_count=?,last_message_at_ms=0 WHERE session_id=?")
        .bind(entries as i64).bind(entries as i64).bind(session.get()).execute(&pool).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&pool)
        .await?;
    if integrity != "ok"
        || sqlx::query("PRAGMA foreign_key_check")
            .fetch_optional(&pool)
            .await?
            .is_some()
    {
        return Err("B06 fixture failed relational integrity checks".into());
    }
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&pool)
        .await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await?;
    let result = json!({"case":"b06","phase":"fixture","history_entries":entries,"session":session.get(),"generation_ms":started.elapsed().as_secs_f64()*1000.0,"sqlite_version":version,"database":stored(&pool,root).await?,"usage_generator":resources::sample(),"integrity":"quick_check + foreign_key_check passed; every user has a succeeded execution and attempt"});
    pool.close().await;
    Ok(result)
}

fn percentile(values: &mut [f64], percent: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[(values.len() * percent).div_ceil(100).saturating_sub(1)])
}

async fn verify_page(
    reader: &KernelReader,
    session: SessionId,
    expected_entry: EntryId,
    expected_len: usize,
) -> AnyResult<Value> {
    let now = Instant::now();
    let page = reader.history(session, None, 100).await?;
    if page.entries.len() != expected_len.min(100)
        || page.entries.last().map(|entry| entry.id) != Some(expected_entry)
        || !page.entries.windows(2).all(|pair| pair[0].id < pair[1].id)
    {
        return Err("B06 branch page lost its shared boundary or entry order".into());
    }
    Ok(
        json!({"session":session,"entries":page.entries.len(),"has_more":page.has_more,"first":page.entries.first().map(|entry|entry.id),"last":page.entries.last().map(|entry|entry.id),"elapsed_ms":now.elapsed().as_secs_f64()*1000.0}),
    )
}

pub(super) async fn run(entries: usize, branches: usize, depth: usize) -> AnyResult<()> {
    if entries == 0 || entries > 1_000_000 || branches == 0 || depth == 0 || branches + depth > 900
    {
        return Err(
            "B06 accepts 1..1000000 entries and positive branch/depth counts totalling at most 900"
                .into(),
        );
    }
    let root = owned_root()?;
    let token = std::fs::read_to_string(root.path().join(MARKER))?;
    let output = std::process::Command::new(std::env::current_exe()?)
        .arg("--history-fixture")
        .arg(root.path())
        .arg(&token)
        .arg(entries.to_string())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "B06 fixture process failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let fixture: Value = serde_json::from_slice(&output.stdout)?;
    let session = SessionId::try_from(
        fixture["session"]
            .as_i64()
            .ok_or("B06 fixture omitted its session")?,
    )?;
    emit(fixture)?;
    let initial = resources::sample();
    let sampler = Sampler::start();
    let started = Instant::now();
    let kernel = open_kernel_runtime(&database_url(root.path())).await?;
    let inspect = inspect(root.path()).await?;
    let (execution, attempt) = claim_in(&kernel, session).await?;
    let round = attempt
        .begin_round(RequestId::new(), vec![execution.message_id])
        .await?
        .value;
    let text = round.begin_text(RequestId::new(), false).await?.value;
    let first = ascii(PACKET_BYTES, 0xb06);
    text.append(&first).await?;
    let boundary = text.entry_id();
    let base = stored(&inspect, root.path()).await?;
    let before_forks = resources::sample();
    let mut children = Vec::with_capacity(branches + depth);
    let mut branch_times = Vec::with_capacity(branches);
    for _ in 0..branches {
        let now = Instant::now();
        let child = kernel
            .ports
            .sessions
            .fork(RequestId::new(), session, boundary)
            .await?
            .value
            .id;
        branch_times.push(now.elapsed().as_secs_f64() * 1000.0);
        children.push(child);
    }
    let after_wide = stored(&inspect, root.path()).await?;
    emit(
        json!({"case":"b06","phase":"wide_forks","history_entries":entries,"branches":branches,"fork_p50_ms":percentile(&mut branch_times,50),"fork_p95_ms":percentile(&mut branch_times,95),"database_before":base,"database_after":after_wide,"io":io_delta(before_forks,resources::sample()),"diagnostics":diagnostics(&kernel),"sampling":sampler.summary()}),
    )?;
    let mut parent = session;
    let mut chain_times = Vec::with_capacity(depth);
    for _ in 0..depth {
        let now = Instant::now();
        parent = kernel
            .ports
            .sessions
            .fork(RequestId::new(), parent, boundary)
            .await?
            .value
            .id;
        chain_times.push(now.elapsed().as_secs_f64() * 1000.0);
        children.push(parent);
    }
    let after_deep = stored(&inspect, root.path()).await?;
    if after_deep["entries"] != base["entries"]
        || after_deep["segment_entries"] != base["segment_entries"]
        || after_deep["segments"] != base["segments"]
    {
        return Err("B06 fork copied immutable history bodies or segment membership".into());
    }
    let origin_edges: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_session_segments WHERE session_id=?")
            .bind(session.get())
            .fetch_one(&inspect)
            .await?;
    let edge_delta = after_deep["session_edges"]
        .as_i64()
        .ok_or("missing edge count")?
        - base["session_edges"]
            .as_i64()
            .ok_or("missing base edge count")?;
    if edge_delta != origin_edges * (branches + depth) as i64 {
        return Err("B06 fork edge cardinality mismatch".into());
    }
    emit(
        json!({"case":"b06","phase":"deep_forks","depth":depth,"deepest_session":parent,"fork_p50_ms":percentile(&mut chain_times,50),"fork_p95_ms":percentile(&mut chain_times,95),"origin_segment_edges":origin_edges,"fork_edges_added":edge_delta,"immutable_bodies_copied":0,"complexity":"each fork copies O(N/128) session segment edges; immutable segment membership and entry bodies stay shared","database":after_deep,"diagnostics":diagnostics(&kernel),"sampling":sampler.summary()}),
    )?;
    let mut watches = Vec::new();
    let mut subscriber_busy = 0;
    for child in &children {
        match kernel.ports.reader.watch_content(*child, text.id()).await {
            Ok(watch) => watches.push((*child, watch)),
            Err(KernelError::Busy) => subscriber_busy += 1,
            Err(error) => return Err(error.into()),
        }
    }
    let second = ascii(PACKET_BYTES, 0xb0602);
    let now = Instant::now();
    text.append(&second).await?;
    let notice = attempt
        .notice(
            RequestId::new(),
            "B06 active shared content checkpoint".into(),
        )
        .await?
        .value;
    let mut notification_times = Vec::new();
    for (_, watch) in &mut watches {
        loop {
            let ready = {
                let head = watch.head()?;
                head.accepted == (first.len() + second.len()) as u64
                    && head.committed == head.accepted
            };
            if ready {
                break;
            }
            tokio::time::timeout(Duration::from_secs(10), watch.changed()).await??;
        }
        notification_times.push(now.elapsed().as_secs_f64() * 1000.0);
    }
    let read_before = kernel.diagnostics.snapshot();
    let mut pages = Vec::new();
    for child in &children {
        let range = kernel
            .ports
            .reader
            .text_range(*child, text.id(), None, 256 * 1024)
            .await?;
        if range.bytes.len() != first.len() + second.len()
            || &range.bytes[..first.len()] != first.as_bytes()
            || &range.bytes[first.len()..] != second.as_bytes()
        {
            return Err("B06 branch did not read the shared active source".into());
        }
        let active = kernel
            .ports
            .reader
            .active_executions(*child, None, 1)
            .await?;
        if !active.executions.is_empty() {
            return Err("B06 inherited an executable attempt".into());
        }
    }
    for child in [session, children[0], parent] {
        pages.push(
            verify_page(
                &kernel.ports.reader,
                child,
                if child == session { notice } else { boundary },
                entries + if child == session { 3 } else { 2 },
            )
            .await?,
        );
    }
    let mut feed_notifications = 0;
    let delivery_started = Instant::now();
    for (index, child) in children.iter().enumerate() {
        let mut watch = kernel.ports.reader.watch(*child, None)?;
        let delivery = kernel
            .ports
            .executor
            .deliver(
                RequestId::new(),
                *child,
                format!("B06-delivery-{index}"),
                json!({"source":text.id(),"branch":child}),
            )
            .await?
            .value;
        let pending = kernel
            .ports
            .reader
            .pending_deliveries(*child, None, 8)
            .await?;
        if pending.len() != 1 || pending[0].id != delivery.id || pending[0].consumed {
            return Err("B06 reliable branch delivery was lost".into());
        }
        if let Some(change) = watch.try_recv()? {
            if change.session != *child {
                return Err("B06 feed crossed a branch scope".into());
            }
            feed_notifications += 1;
        } else {
            return Err("B06 committed branch delivery did not notify its subscriber".into());
        }
    }
    emit(
        json!({"case":"b06","phase":"shared_live_output","branches_read":children.len(),"content_subscribers_admitted":watches.len(),"content_subscribers_busy":subscriber_busy,"subscriber_policy":"source admission is bounded; rejected branches still use authorized range reads","notification_p50_ms":percentile(&mut notification_times,50),"notification_p95_ms":percentile(&mut notification_times,95),"reliable_deliveries":children.len(),"business_feed_notifications":feed_notifications,"deliveries_ms":delivery_started.elapsed().as_secs_f64()*1000.0,"page_reads":pages,"read_counters":metric_delta(&read_before,&kernel.diagnostics.snapshot())?,"database":stored(&inspect,root.path()).await?,"diagnostics":diagnostics(&kernel),"sampling":sampler.summary()}),
    )?;
    let snapshot = kernel.ports.reader.history_snapshot(children[0]).await?;
    let delete_started = Instant::now();
    kernel
        .ports
        .sessions
        .delete(RequestId::new(), session)
        .await?;
    let origin_delete_ms = delete_started.elapsed().as_secs_f64() * 1000.0;
    for child in &children {
        let range = kernel
            .ports
            .reader
            .text_range(*child, text.id(), None, 256 * 1024)
            .await?;
        if range.bytes.len() != first.len() + second.len()
            || range.stream.state != CaptureState::Interrupted
        {
            return Err("B06 origin deletion lost shared content or left capture open".into());
        }
    }
    let gc_pinned = kernel.maintenance.collect_history_snapshots().await?;
    drop(snapshot);
    let gc_released = kernel.maintenance.collect_history_snapshots().await?;
    let blobs = kernel.maintenance.collect_blobs().await?;
    let delete_branch_started = Instant::now();
    kernel
        .ports
        .sessions
        .delete(RequestId::new(), children[0])
        .await?;
    let branch_delete_ms = delete_branch_started.elapsed().as_secs_f64() * 1000.0;
    let retained = kernel
        .ports
        .reader
        .text_range(parent, text.id(), None, 256 * 1024)
        .await?;
    if retained.bytes.len() != first.len() + second.len() {
        return Err("B06 sibling deletion lost the deepest branch source".into());
    }
    drop(retained);
    emit(
        json!({"case":"b06","phase":"delete_and_gc","origin_delete_ms":origin_delete_ms,"branch_delete_ms":branch_delete_ms,"shared_source_state":"interrupted","snapshot_gc_while_pinned":{"roots":gc_pinned.roots,"references":gc_pinned.references,"segments":gc_pinned.segments},"snapshot_gc_after_release":{"roots":gc_released.roots,"references":gc_released.references,"segments":gc_released.segments},"blob_gc":{"retired":blobs.retired,"chunks":blobs.chunks,"blobs":blobs.blobs},"gc_scope":"snapshot roots/retired segment references and unreferenced blobs; this does not claim whole-history entry compaction","database":stored(&inspect,root.path()).await?,"diagnostics":diagnostics(&kernel),"sampling":sampler.summary()}),
    )?;
    drop((watches, text, round, attempt));
    let final_diagnostics = kernel.diagnostics.clone();
    kernel.ports.clone().shutdown().await?;
    inspect.close().await;
    drop(kernel);
    emit(
        json!({"case":"b06","phase":"closed","elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"sampling":sampler.summary(),"usage":resources::sample(),"total_io":io_delta(initial,resources::sample()),"read_budget_after_close":final_diagnostics.read_budget(),"wal_checkpoint_after_close":closed_checkpoint(root.path()).await?}),
    )?;
    Ok(())
}
