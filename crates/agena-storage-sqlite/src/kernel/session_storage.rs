//! Owner-attributed session storage and explicit, resumable cleanup.

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use agena_storage_layout::StorageLayout;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::*;

mod accounting;
mod files;
mod purge;

use accounting::{MeasuredSession, Recording};
use files::File;

const JOB_PREFIX: &str = "session-storage/";
const PAGE_LIMIT: usize = 100;
const FILES_PER_BATCH: usize = 16;
/// Upper bound on live ephemeral ACP views kept in the in-memory SQLite kernel.
const MAX_ACP_VIEWS: i64 = 128;
const ACP_RECLAIM_BATCH: i64 = 16;
static FILE_WORKERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(Default)]
pub(super) struct InventoryCache(Mutex<Option<(Instant, Arc<Inventory>)>>);

struct Inventory {
    sessions: Vec<SessionStorageSummary>,
    bots: Vec<StorageBot>,
    shared: SharedStorage,
    measured_at_ms: i64,
}

impl InventoryCache {
    pub async fn invalidate(&self) {
        *self.0.lock().await = None;
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Job {
    session: SessionId,
    status: StorageOperation,
    files: Vec<File>,
    recordings: Vec<Recording>,
    database_pending: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    next_attempt_ms: i64,
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

struct Plan {
    preview: StorageCleanupPreview,
    measured: MeasuredSession,
    files: Vec<File>,
    recordings: Vec<Recording>,
}

fn key(session: SessionId) -> String {
    format!("{JOB_PREFIX}{}", session.get())
}

async fn find_job(connection: &mut SqliteConnection, session: SessionId) -> Result<Option<Job>> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value_json FROM agena_server_state WHERE key=?")
            .bind(key(session))
            .fetch_optional(connection)
            .await
            .map_err(sql_error)?;
    value.as_deref().map(decode).transpose()
}

async fn save(connection: &mut SqliteConnection, job: &Job) -> Result<()> {
    sqlx::query(
        "INSERT INTO agena_server_state (key,value_json)
         VALUES (?,json_set(?,'$.updated_at_ms',?))
         ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
    )
    .bind(key(job.session))
    .bind(encode(job)?)
    .bind(now_ms())
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn database_path(connection: &mut SqliteConnection) -> Result<Option<PathBuf>> {
    let path: String =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name='main'")
            .fetch_one(connection)
            .await
            .map_err(sql_error)?;
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

fn layout(path: &Path) -> Result<StorageLayout> {
    let root = path
        .parent()
        .ok_or_else(|| storage("database has no data root"))?;
    Ok(StorageLayout::new(root))
}

async fn scan(path: Option<&Path>, session: SessionId) -> Result<Vec<File>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let layout = layout(path)?;
    FILE_WORKERS
        .run(move || match files::Scanner::open(layout)? {
            Some(scanner) => scanner.session(session.get()),
            None => Ok(Vec::new()),
        })
        .await
        .map_err(storage)?
        .map_err(storage)
}

async fn session_state(
    connection: &mut SqliteConnection,
    session: SessionId,
    include_payloads: bool,
    active_blob_tokens: &[String],
) -> Result<MeasuredSession> {
    accounting::measure(
        connection,
        Some(session),
        include_payloads,
        active_blob_tokens,
    )
    .await?
    .pop()
    .ok_or_else(|| KernelError::NotFound("session storage".into()))
}

async fn database_storage(connection: &mut SqliteConnection) -> Result<DatabaseStorage> {
    let path = database_path(connection).await?;
    let (size, free): (i64, i64) = sqlx::query_as(
        "SELECT page_size,freelist_count FROM pragma_page_size(),pragma_freelist_count()",
    )
    .fetch_one(connection)
    .await
    .map_err(sql_error)?;
    let reusable_bytes = accounting::unsigned(size).saturating_mul(accounting::unsigned(free));
    let Some(path) = path else {
        return Ok(DatabaseStorage {
            file_bytes: 0,
            wal_bytes: 0,
            reusable_bytes,
            can_reclaim: false,
        });
    };
    FILE_WORKERS
        .run(move || {
            let mut wal = path.as_os_str().to_os_string();
            wal.push("-wal");
            let wal_bytes =
                std::fs::metadata(PathBuf::from(wal)).map_or(0, |metadata| metadata.len());
            Ok(DatabaseStorage {
                file_bytes: std::fs::metadata(path).map_err(storage)?.len(),
                wal_bytes,
                reusable_bytes,
                can_reclaim: reusable_bytes > 0 || wal_bytes > 0,
            })
        })
        .await
        .map_err(storage)?
}

async fn fill_inventory(database: &DatabaseConnection, pins: &blobs::Pins) -> Result<Inventory> {
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let path = database_path(&mut connection).await?;
    let mut sessions =
        accounting::measure(&mut connection, None, true, &pins.active_tokens()).await?;
    let mut bots = accounting::bots(&mut connection).await?;
    let purged: Vec<i64> = sqlx::query_scalar(
        r#"
        SELECT json_extract(value_json,'$.session') FROM agena_server_state
        WHERE key GLOB 'session-storage/*' AND json_extract(value_json,'$.status.purge')=1
          AND json_extract(value_json,'$.status.state')='completed'
    "#,
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let bindings: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,owner_bot_id FROM agena_integration_bindings WHERE owner_bot_id IS NOT NULL",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    drop(connection);

    let root = path.as_deref().map(layout).transpose()?;
    let purged: BTreeSet<_> = purged.into_iter().collect();
    let (mut rows, shared, channel_files) = FILE_WORKERS
        .run(move || {
            let scan = root.map(files::Scanner::open).transpose()?.flatten();
            let mut result = Vec::with_capacity(sessions.len());
            for mut session in sessions.drain(..) {
                if let Some(scan) = &scan {
                    let found = scan.session(session.summary.session_id.get())?;
                    accounting::count_files(&mut session.summary, &found);
                }
                if purged.contains(&session.summary.session_id.get())
                    && session.summary.bytes.total() == session.retained_bytes
                {
                    continue;
                }
                result.push(session.summary);
            }
            let mut channel_files = HashMap::<_, u64>::new();
            if let Some(scan) = &scan {
                for (binding, owner) in bindings {
                    let bytes = channel_files.entry(owner).or_default();
                    *bytes = bytes.saturating_add(scan.channel(&binding)?);
                }
            }
            let shared = scan
                .as_ref()
                .map(files::Scanner::shared)
                .transpose()?
                .unwrap_or_default();
            Ok::<_, std::io::Error>((result, shared, channel_files))
        })
        .await
        .map_err(storage)?
        .map_err(storage)?;
    rows.sort_unstable_by_key(|row| std::cmp::Reverse((row.bytes.total(), row.session_id)));

    let mut totals = HashMap::<_, (usize, u64)>::new();
    for row in &rows {
        if let Some(id) = &row.bot_id {
            let (count, bytes) = totals.entry(id).or_default();
            *count += 1;
            *bytes = bytes.saturating_add(row.bytes.total());
        }
    }
    for bot in &mut bots {
        (bot.session_count, bot.session_bytes) = totals.remove(&bot.id).unwrap_or_default();
        bot.channel_files = channel_files.get(&bot.id).copied().unwrap_or_default();
    }
    Ok(Inventory {
        sessions: rows,
        bots,
        shared,
        measured_at_ms: now_ms(),
    })
}

pub(super) async fn list(
    database: &DatabaseConnection,
    cache: &InventoryCache,
    query: SessionStorageQuery,
    pins: &blobs::Pins,
) -> Result<SessionStorageList> {
    let inventory = {
        let mut cached = cache.0.lock().await;
        if query.refresh == Some(true)
            || cached
                .as_ref()
                .is_none_or(|(time, _)| time.elapsed() > Duration::from_secs(10))
        {
            *cached = Some((
                Instant::now(),
                Arc::new(fill_inventory(database, pins).await?),
            ));
        }
        cached.as_ref().expect("inventory is populated").1.clone()
    };
    let search = query
        .search
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let matches = |row: &&SessionStorageSummary| {
        query
            .bot
            .as_ref()
            .is_none_or(|bot| row.bot_id.as_ref() == Some(bot))
            && (search.is_empty()
                || row.title.to_lowercase().contains(&search)
                || row
                    .workspace_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&search))
                || row
                    .bot_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&search))
                || row.session_id.get().to_string() == search)
            && match query.scope {
                None => true,
                Some(SessionStorageScope::Deleted) => row.deleted,
                Some(SessionStorageScope::Project) => {
                    row.workspace_id.is_some() && row.bot_id.is_none() && !row.deleted
                }
                Some(SessionStorageScope::Bot) => row.bot_id.is_some() && !row.deleted,
            }
    };
    let filtered = inventory.sessions.iter().filter(matches);
    let (total, bytes) = filtered.clone().fold((0, 0u64), |(count, bytes), row| {
        (count + 1, bytes.saturating_add(row.bytes.total()))
    });
    let items = filtered
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(40).clamp(1, PAGE_LIMIT))
        .cloned()
        .collect();
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    Ok(SessionStorageList {
        items,
        total,
        bytes,
        measured_at_ms: inventory.measured_at_ms,
        bots: inventory.bots.clone(),
        shared: inventory.shared,
        database: database_storage(&mut connection).await?,
    })
}

async fn file_protection(
    connection: &mut SqliteConnection,
    owner: &MeasuredSession,
    file: &File,
    purge: bool,
) -> Result<Option<StorageProtection>> {
    if file.kind == SessionFileKind::Cache {
        return Ok(None);
    }
    if owner.shared_history {
        return Ok(Some(StorageProtection::Shared));
    }
    if !purge && file.kind == SessionFileKind::Plugin {
        return Ok(Some(StorageProtection::Unmanaged));
    }
    let except = purge.then_some(owner.summary.session_id);
    let referenced = accounting::referenced(connection, file.name(), except).await?;
    Ok(referenced.then_some(StorageProtection::Referenced))
}

pub(super) async fn operation(
    database: &DatabaseConnection,
    session: SessionId,
) -> Result<Option<StorageOperation>> {
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let value: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(value_json,'$.status') FROM agena_server_state WHERE key=?",
    )
    .bind(key(session))
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    value.as_deref().map(decode).transpose()
}

pub(super) async fn detail(
    database: &DatabaseConnection,
    session: SessionId,
    query: SessionStorageDetailQuery,
    pins: &blobs::Pins,
) -> Result<SessionStorageDetail> {
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let path = database_path(&mut connection).await?;
    let mut owner = session_state(&mut connection, session, true, &pins.active_tokens()).await?;
    let operation = find_job(&mut connection, session)
        .await?
        .map(|job| job.status);
    let removable_recording_bytes = if owner.summary.busy_reason.is_none() && !owner.summary.deleted
    {
        accounting::recordings(&mut connection, session)
            .await?
            .iter()
            .map(|row| row.bytes)
            .sum()
    } else {
        0
    };
    drop(connection);
    let mut assets = scan(path.as_deref(), session).await?;
    accounting::count_files(&mut owner.summary, &assets);
    let file_count = assets.len();
    assets.sort_by_key(|file| std::cmp::Reverse(file.bytes()));
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let mut found = Vec::new();
    let asset_page = assets.iter().skip(query.file_offset.unwrap_or(0)).take(
        query
            .file_limit
            .unwrap_or(accounting::FILE_PAGE_SIZE)
            .clamp(1, PAGE_LIMIT),
    );
    for file in asset_page {
        let protection = match (owner.summary.busy_reason, file.kind) {
            (Some(reason), _) => Some(reason),
            (None, SessionFileKind::Cache) => Some(StorageProtection::Unmanaged),
            (None, _) => file_protection(&mut connection, &owner, file, false).await?,
        };
        found.push(file.resource(protection));
    }
    Ok(SessionStorageDetail {
        summary: owner.summary,
        shared_history: owner.shared_history,
        files: found,
        file_count,
        removable_recording_bytes,
        operation,
    })
}

async fn plan(
    connection: &mut SqliteConnection,
    session: SessionId,
    mut selection: StorageSelection,
    pins: &blobs::Pins,
) -> Result<Plan> {
    selection.files.sort_unstable();
    selection.files.dedup();
    if selection.files.len() > accounting::MAX_SELECTED_FILES
        || selection
            .files
            .iter()
            .any(|value| value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid("invalid cleanup file selection"));
    }
    let path = database_path(connection).await?;
    let owner = session_state(connection, session, selection.purge, &pins.active_tokens()).await?;
    let protection = owner.summary.busy_reason;
    let mut candidates = Vec::new();
    let mut selected = BTreeSet::new();
    if protection.is_none() {
        for file in scan(path.as_deref(), session).await? {
            let explicitly_selected = !selection.purge
                && !selection.files.is_empty()
                && selection.files.binary_search(&file.id()).is_ok();
            if !(selection.purge
                || (selection.cache && file.kind == SessionFileKind::Cache)
                || explicitly_selected)
            {
                continue;
            }
            if explicitly_selected && file.kind == SessionFileKind::Cache {
                return Err(invalid(
                    "choose the cache category to remove all related files together",
                ));
            }
            if let Some(reason) =
                file_protection(connection, &owner, &file, selection.purge).await?
            {
                if explicitly_selected {
                    return Err(KernelError::Conflict(format!(
                        "cleanup file has a protected reference ({reason:?})"
                    )));
                }
                continue;
            }
            if explicitly_selected {
                selected.insert(file.id());
            }
            candidates.push(file);
        }
        if !selection.purge && selected.len() != selection.files.len() {
            return Err(KernelError::Conflict(
                "a selected file changed; refresh storage usage".into(),
            ));
        }
    }
    let recordings = if !selection.purge
        && selection.recordings
        && protection.is_none()
        && !owner.summary.deleted
    {
        accounting::recordings(connection, session).await?
    } else {
        Vec::new()
    };
    let file_bytes = candidates.iter().map(File::bytes).sum();
    let database_bytes = if selection.purge && !owner.shared_history && protection.is_none() {
        owner
            .summary
            .bytes
            .total()
            .saturating_sub(owner.retained_bytes)
    } else {
        recordings.iter().map(|recording| recording.bytes).sum()
    };
    let previous = find_job(connection, session)
        .await?
        .map(|job| job.status.id);
    let digest = encode(&(
        session,
        &selection,
        owner.version,
        owner.summary.deleted,
        owner.shared_history,
        protection,
        database_bytes,
        &candidates,
        &recordings,
        previous,
    ))?;
    let token = hex::encode(Sha256::digest(digest));
    let preview = StorageCleanupPreview {
        token,
        file_bytes,
        database_bytes,
        file_count: candidates.len(),
        keeps_shared_history: selection.purge && owner.shared_history,
        protection,
    };
    Ok(Plan {
        preview,
        measured: owner,
        files: candidates,
        recordings,
    })
}

pub(super) async fn preview(
    database: &DatabaseConnection,
    session: SessionId,
    selection: StorageSelection,
    pins: &blobs::Pins,
) -> Result<StorageCleanupPreview> {
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    Ok(plan(&mut connection, session, selection, pins)
        .await?
        .preview)
}

pub(super) async fn confirm(
    database: &KernelDatabase,
    session: SessionId,
    confirmation: ConfirmStorageCleanup,
    feed: &ChangeFeed,
    cache: &Arc<read_cache::ReadCache>,
    pins: &blobs::Pins,
) -> Result<StorageOperation> {
    let _permit = crate::acquire_write_permit(&database.writer)
        .await
        .map_err(storage)?;
    let pool = database.writer.get_sqlite_connection_pool();
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(sql_error)?;
    if let Some(previous) = find_job(&mut transaction, session).await? {
        if previous.status.id == confirmation.token {
            return Ok(previous.status);
        }
        if previous.status.state == StorageOperationState::Running {
            return Err(KernelError::Conflict(
                "a cleanup is already running for this session".into(),
            ));
        }
    }
    let selection = confirmation.selection;
    let planned = plan(&mut transaction, session, selection.clone(), pins).await?;
    if planned.preview.token != confirmation.token {
        return Err(KernelError::Conflict(
            "storage changed; preview the cleanup again".into(),
        ));
    }
    if planned.preview.protection.is_some() {
        return Err(KernelError::Conflict(
            "session is running or has an open history reader".into(),
        ));
    }
    if !selection.purge && planned.files.is_empty() && planned.recordings.is_empty() {
        return Err(invalid("there are no selected items eligible for cleanup"));
    }
    let _fence = cache.pause();
    let tombstoned = selection.purge && !planned.measured.summary.deleted;
    if tombstoned {
        lifecycle::delete(&mut transaction, session).await?;
    }
    let job = Job {
        session,
        status: StorageOperation {
            id: confirmation.token,
            state: StorageOperationState::Running,
            purge: selection.purge,
            file_bytes_removed: 0,
            files_removed: 0,
            files_skipped: 0,
            database_bytes_removed: 0,
            keeps_shared_history: planned.preview.keeps_shared_history,
            detail: None,
        },
        files: planned.files,
        recordings: planned.recordings,
        database_pending: selection.purge,
        next_attempt_ms: 0,
    };
    save(&mut transaction, &job).await?;
    transaction.commit().await.map_err(sql_error)?;
    if tombstoned {
        cache.clear();
        feed.invalidate();
    }
    Ok(job.status)
}

async fn next_job(connection: &mut SqliteConnection) -> Result<Option<Job>> {
    let value: Option<String> = sqlx::query_scalar(
        r#"
        SELECT value_json FROM agena_server_state WHERE key GLOB 'session-storage/*'
          AND json_extract(value_json,'$.status.state')='running'
          AND COALESCE(json_extract(value_json,'$.next_attempt_ms'),0)<=?
        ORDER BY json_extract(value_json,'$.updated_at_ms'),key LIMIT 1
    "#,
    )
    .bind(now_ms())
    .fetch_optional(connection)
    .await
    .map_err(sql_error)?;
    value.as_deref().map(decode).transpose()
}

pub(super) async fn step(
    database: &KernelDatabase,
    feed: &ChangeFeed,
    cache: &Arc<read_cache::ReadCache>,
    frame_reads: &Semaphore,
    pins: &blobs::Pins,
) -> Result<Option<Duration>> {
    let pool = database.writer.get_sqlite_connection_pool();
    let mut connection = pool.acquire().await.map_err(storage)?;
    let Some(mut job) = next_job(&mut connection).await? else {
        let next: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT MIN(COALESCE(json_extract(value_json,'$.next_attempt_ms'),0))
            FROM agena_server_state WHERE key GLOB 'session-storage/*'
              AND json_extract(value_json,'$.status.state')='running'
        "#,
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        return Ok(next.map(|time| {
            Duration::from_millis(
                accounting::unsigned(time.saturating_sub(now_ms())).clamp(125, 10_000),
            )
        }));
    };
    drop(connection);
    let _permit = crate::acquire_write_permit(&database.writer)
        .await
        .map_err(storage)?;
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(sql_error)?;
    let owner = session_state(&mut transaction, job.session, false, &pins.active_tokens()).await?;
    let frame_slot = if job.database_pending || !job.recordings.is_empty() {
        frame_reads.try_acquire().ok()
    } else {
        None
    };
    if owner.summary.busy_reason == Some(StorageProtection::Reading)
        || ((job.database_pending || !job.recordings.is_empty()) && frame_slot.is_none())
    {
        job.next_attempt_ms = now_ms().saturating_add(1_500);
        save(&mut transaction, &job).await?;
        transaction.commit().await.map_err(sql_error)?;
        return Ok(Some(Duration::from_millis(125)));
    }
    job.next_attempt_ms = 0;
    let mut changed_history = false;
    if owner.summary.busy {
        job.status.state = StorageOperationState::Failed;
        job.status.detail = Some("session_changed".into());
    } else if job.database_pending {
        let savepoint = transaction.begin().await.map_err(sql_error)?;
        match purge::reclaim(savepoint, job.session, owner.shared_history).await {
            Ok((removed, pending)) => {
                job.status.database_bytes_removed += removed;
                job.status.keeps_shared_history = owner.shared_history;
                job.database_pending = pending;
            }
            Err(error) => {
                tracing::warn!(%error, session=%job.session.get(), "could not reclaim a deleted session");
                job.status.state = StorageOperationState::Failed;
                job.status.detail = Some("database_cleanup_failed".into());
            }
        }
    } else if let Some(recording) = job.recordings.first() {
        let shared = accounting::recording_shared(&mut transaction, recording.id).await?;
        if shared || owner.summary.deleted {
            job.status.state = StorageOperationState::Failed;
            job.status.detail = Some("session_changed".into());
        } else {
            let mut savepoint = transaction.begin().await.map_err(sql_error)?;
            match content::prune_measured(
                &mut savepoint,
                job.session,
                recording.id,
                recording.before,
            )
            .await
            {
                Ok((retention, removed)) => {
                    savepoint.commit().await.map_err(sql_error)?;
                    job.status.database_bytes_removed += removed;
                    if !retention.pending_reclamation {
                        job.recordings.remove(0);
                    }
                    changed_history = true;
                }
                Err(error) => {
                    savepoint.rollback().await.map_err(sql_error)?;
                    tracing::warn!(%error, session=%job.session.get(), "could not trim session recordings");
                    job.status.state = StorageOperationState::Failed;
                    job.status.detail = Some("database_cleanup_failed".into());
                }
            }
        }
    } else if !job.files.is_empty() {
        let path = database_path(&mut transaction)
            .await?
            .ok_or_else(|| storage("no managed file root"))?;
        let root = layout(&path)?;
        let cache_path = root.session_web_cache_dir(job.session.get());
        let cache_relative = cache_path.strip_prefix(root.root()).map_err(storage)?;
        let (live_cache, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut job.files)
            .into_iter()
            .partition(|file| {
                file.kind == SessionFileKind::Cache && file.path.starts_with(cache_relative)
            });
        job.files = remaining;
        if live_cache.is_empty() {
            for file in job.files.drain(..job.files.len().min(FILES_PER_BATCH)) {
                if file_protection(&mut transaction, &owner, &file, job.status.purge)
                    .await?
                    .is_some()
                {
                    job.status.files_skipped += 1;
                    continue;
                }
                let bytes = file.bytes();
                match FILE_WORKERS
                    .run({
                        let root = root.clone();
                        move || files::unlink(&root, &file)
                    })
                    .await
                    .map_err(storage)?
                {
                    Ok(files::Removal::Removed) => {
                        job.status.files_removed += 1;
                        job.status.file_bytes_removed += bytes;
                    }
                    Ok(files::Removal::Gone) => {}
                    Ok(files::Removal::Changed) => job.status.files_skipped += 1,
                    Err(error) => {
                        tracing::warn!(%error, session=%job.session.get(), "managed cleanup file could not be removed");
                        job.status.files_skipped += 1;
                    }
                }
            }
        } else {
            let token = job.status.id.clone();
            let session = job.session.get();
            let count = live_cache.len();
            match FILE_WORKERS
                .run(move || files::park_cache(&root, session, &token, &live_cache))
                .await
                .map_err(storage)?
            {
                Ok(Some(files)) => job.files.extend(files),
                Ok(None) => job.status.files_skipped += count,
                Err(error) => {
                    tracing::warn!(%error, session, "could not move the session cache for cleanup");
                    job.status.files_skipped += count;
                }
            }
        }
    }
    if job.status.state == StorageOperationState::Running
        && !job.database_pending
        && job.recordings.is_empty()
        && job.files.is_empty()
    {
        job.status.state = StorageOperationState::Completed;
    }
    if job.status.state != StorageOperationState::Running {
        job.files.clear();
        job.recordings.clear();
        job.database_pending = false;
    }
    save(&mut transaction, &job).await?;
    let _fence = changed_history.then(|| cache.pause());
    transaction.commit().await.map_err(sql_error)?;
    if changed_history {
        cache.clear();
        feed.invalidate();
    }
    // Another session may already have a job ready, even if this one finished.
    Ok(Some(Duration::from_millis(125)))
}

pub(super) async fn reclaim_database(
    database: &KernelDatabase,
    pins: &blobs::Pins,
) -> Result<DatabaseReclaimResult> {
    let _permit = crate::acquire_write_permit(&database.writer)
        .await
        .map_err(storage)?;
    let mut connection = database
        .writer
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let Some(path) = database_path(&mut connection).await? else {
        return Err(invalid("an in-memory database has no disk file"));
    };
    let busy: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(SELECT 1 FROM agena_attempts WHERE phase IN ('queued','running'))
          OR EXISTS(SELECT 1 FROM agena_background_activities WHERE phase='running')
          OR EXISTS(SELECT 1 FROM agena_history_snapshots)
          OR EXISTS(SELECT 1 FROM json_each(?))
          OR EXISTS(SELECT 1 FROM agena_server_state WHERE key GLOB 'session-storage/*'
            AND json_extract(value_json,'$.status.state')='running')
    "#,
    )
    .bind(encode(&pins.active_tokens())?)
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    if busy {
        return Err(KernelError::Conflict(
            "database storage can be reclaimed when execution and cleanup are idle".into(),
        ));
    }
    let before = database_storage(&mut connection).await?;
    checkpoint(&mut connection).await?;
    if before.reusable_bytes > 0 {
        let checkpointed = database_storage(&mut connection).await?;
        let available = FILE_WORKERS
            .run(move || fs4::available_space(&path))
            .await
            .map_err(storage)?
            .map_err(storage)?;
        if available < checkpointed.file_bytes.saturating_mul(2) {
            return Err(KernelError::Conflict(
                "there is not enough temporary disk space to reclaim the database".into(),
            ));
        }
        sqlx::query("VACUUM")
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
        checkpoint(&mut connection).await?;
    }
    let database = database_storage(&mut connection).await?;
    let released_bytes = before
        .file_bytes
        .saturating_add(before.wal_bytes)
        .saturating_sub(database.file_bytes.saturating_add(database.wal_bytes));
    Ok(DatabaseReclaimResult {
        database,
        released_bytes,
    })
}

/// Evict excess idle ACP views and physically purge soft-deleted ACP views
/// inside the ephemeral in-memory kernel so RAM stays bounded across repeated
/// `Open`/`replace_view` and session creation cycles.
pub(super) async fn collect_acp_views(
    database: &KernelDatabase,
    feed: &Arc<ChangeFeed>,
    cache: &Arc<read_cache::ReadCache>,
    pins: &blobs::Pins,
) -> Result<u64> {
    let _permit = crate::acquire_write_permit(&database.writer)
        .await
        .map_err(storage)?;
    let mut transaction = database
        .writer
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let active_tokens = pins.active_tokens();
    let mut changed = false;
    let mut reclaimed = 0u64;

    // 1. Soft-delete excess idle live views beyond MAX_ACP_VIEWS (oldest first).
    let overflow: Vec<i64> = sqlx::query_scalar(
        r#"
        SELECT s.id FROM agena_sessions s
        WHERE s.deleted_at_ms IS NULL
          AND s.id NOT IN (
              SELECT id FROM agena_sessions
              WHERE deleted_at_ms IS NULL
              ORDER BY updated_at_ms DESC, id DESC
              LIMIT ?
          )
        ORDER BY s.updated_at_ms ASC, s.id ASC
        LIMIT ?
        "#,
    )
    .bind(MAX_ACP_VIEWS)
    .bind(ACP_RECLAIM_BATCH)
    .fetch_all(&mut *transaction)
    .await
    .map_err(sql_error)?;
    for id in overflow {
        let session = SessionId::try_from(id).map_err(storage)?;
        let Ok(owner) = session_state(&mut transaction, session, false, &active_tokens).await
        else {
            continue;
        };
        if owner.summary.busy {
            continue;
        }
        let has_pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agena_interactions WHERE session_id=? AND decision_json IS NULL)",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(sql_error)?;
        if has_pending {
            continue;
        }
        lifecycle::delete(&mut transaction, session).await?;
        changed = true;
    }

    // 2. Physically reclaim payloads and rows of soft-deleted ACP views.
    let deleted: Vec<i64> = sqlx::query_scalar(
        r#"
        SELECT s.id FROM agena_sessions s
        WHERE s.deleted_at_ms IS NOT NULL
          AND NOT EXISTS(SELECT 1 FROM agena_history_snapshots h WHERE h.session_id=s.id)
        ORDER BY s.deleted_at_ms ASC, s.id ASC
        LIMIT ?
        "#,
    )
    .bind(ACP_RECLAIM_BATCH)
    .fetch_all(&mut *transaction)
    .await
    .map_err(sql_error)?;
    for id in deleted {
        let session = SessionId::try_from(id).map_err(storage)?;
        let Ok(owner) = session_state(&mut transaction, session, false, &active_tokens).await
        else {
            continue;
        };
        if owner.summary.busy {
            continue;
        }
        let mut purged_all = false;
        for _ in 0..64 {
            let savepoint = transaction.begin().await.map_err(sql_error)?;
            match purge::reclaim(savepoint, session, owner.shared_history).await {
                Ok((removed, pending)) => {
                    reclaimed += removed;
                    changed = true;
                    if !pending {
                        purged_all = true;
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, session = id, "deferred ephemeral ACP view reclamation");
                    break;
                }
            }
        }
        if purged_all && !owner.shared_history {
            let referenced: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS(SELECT 1 FROM agena_sessions WHERE (parent_session_id=?1 OR root_session_id=?1) AND id != ?1)
                  OR EXISTS(SELECT 1 FROM agena_entries WHERE origin_session_id=?1)
                  OR EXISTS(SELECT 1 FROM agena_blobs WHERE session_id=?1)
                  OR EXISTS(SELECT 1 FROM agena_branch_sources WHERE session_id=?1)
                  OR EXISTS(SELECT 1 FROM agena_history_snapshots WHERE session_id=?1)
                "#,
            )
            .bind(id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(sql_error)?;
            if !referenced {
                let mut sp = transaction.begin().await.map_err(sql_error)?;
                let deleted_row = sqlx::query("DELETE FROM agena_sessions WHERE id=?")
                    .bind(id)
                    .execute(&mut *sp)
                    .await;
                if deleted_row.is_ok() {
                    let _ = sp.commit().await;
                    changed = true;
                } else {
                    let _ = sp.rollback().await;
                }
            }
        }
    }

    let _fence = changed.then(|| cache.pause());
    transaction.commit().await.map_err(sql_error)?;
    if changed {
        cache.clear();
        feed.invalidate();
    }
    Ok(reclaimed)
}

async fn checkpoint(connection: &mut SqliteConnection) -> Result<()> {
    let (busy, _, _): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    if busy != 0 {
        return Err(KernelError::Conflict(
            "an active reader is holding the SQLite journal".into(),
        ));
    }
    Ok(())
}
