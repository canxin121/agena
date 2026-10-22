//! Durable session command processor. Commands enter one byte-bounded queue;
//! a batch uses one real SQLite transaction and per-command savepoints.

use std::{sync::Arc, time::Duration};

use agena_domain::{ContentEpoch, ContentId, ExecutionId, KernelEpoch};
use agena_storage::{
    backend::kernel::{self as port, ChangeFeed, Command, KernelBackend, KernelPorts, Mutation},
    kernel::*,
};
use async_trait::async_trait;
use sea_orm::sqlx::{self, Connection, Row, SqliteConnection};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

use crate::KernelDatabase;

mod activity;
#[cfg(test)]
mod activity_tests;
mod archive;
#[cfg(test)]
mod archive_tests;
#[cfg(test)]
mod blob_tests;
mod blobs;
#[cfg(test)]
mod capture_tests;
mod commands;
mod content;
#[cfg(test)]
mod contract_completion_tests;
mod control_snapshot;
#[cfg(test)]
mod control_snapshot_tests;
#[cfg(test)]
mod crash_window_tests;
mod import;
#[cfg(test)]
mod import_tests;
mod input_window;
#[cfg(test)]
mod input_window_tests;
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod loss_tests;
mod metrics;
#[cfg(test)]
mod owned_tool_tests;
mod owned_tools;
mod prompt;
#[cfg(test)]
mod prompt_tests;
#[cfg(test)]
mod tool_draft_tests;
pub use metrics::{KernelDiagnostics, KernelMetricsSnapshot};
#[cfg(test)]
mod content_tests;
#[cfg(test)]
mod event_cache_tests;
#[cfg(test)]
mod event_tests;
mod history_views;
mod read_cache;
#[cfg(test)]
mod read_tests;
mod reads;
pub use read_cache::ReadCacheSnapshot;
#[cfg(test)]
mod retention_tests;
pub(crate) mod schema;
#[cfg(test)]
mod snapshot_tests;
mod snapshots;
#[cfg(test)]
mod state_machine_tests;
mod stop;
#[cfg(test)]
mod stop_tests;
#[cfg(test)]
mod summary_tests;
#[cfg(test)]
mod tests;

const MAX_PENDING_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONTENT_PENDING_BYTES: usize = 14 * 1024 * 1024;
const MAX_COMMAND_BYTES: usize = 1024 * 1024;
const MAX_BATCH_COMMANDS: usize = 64;
const MAX_BATCH_BYTES: usize = 512 * 1024;

type Reply = Result<CommitReceipt<Mutation>>;

struct Pending {
    request: RequestId,
    command: Command,
    digest: Vec<u8>,
    bytes: usize,
    response: oneshot::Sender<Reply>,
    _permit: OwnedSemaphorePermit,
    _retention_read: Option<OwnedSemaphorePermit>,
}

struct Backend {
    reader: Arc<sea_orm::DatabaseConnection>,
    sender: mpsc::Sender<Work>,
    budget: Arc<Semaphore>,
    feed: Arc<ChangeFeed>,
    live: Arc<content::LiveContent>,
    content_budget: Arc<Semaphore>,
    content_queue_slots: Arc<Semaphore>,
    content_reads: Arc<Semaphore>,
    control_reads: Arc<Semaphore>,
    frame_budget: Arc<Semaphore>,
    frame_reads: Arc<Semaphore>,
    blob_pins: Arc<blobs::Pins>,
    blob_uploads: Arc<Semaphore>,
    cache: Arc<read_cache::ReadCache>,
    views: ReadBudget,
    history_pins: Arc<snapshots::Pins>,
    snapshot_opens: Arc<Semaphore>,
    exports: archive::Exports,
    imports: Arc<Semaphore>,
}

enum Work {
    ImportArchive(Box<import::Job>),
    OpenHistorySnapshot {
        session: SessionId,
        epoch: KernelEpoch,
        response: oneshot::Sender<Result<port::HistorySnapshotOpened>>,
    },
    CollectHistorySnapshots(oneshot::Sender<Result<port::HistorySnapshotGc>>),
    Command(Box<Pending>),
    Append(Box<content::Append>),
    AppendEvent(Box<content::events::AppendEvent>),
    WatchContent {
        session: SessionId,
        content: ContentId,
        response: oneshot::Sender<Result<ContentWatch>>,
    },
    Shutdown(oneshot::Sender<Result<()>>),
    CollectBlobs(oneshot::Sender<Result<port::BlobGc>>),
}

/// Open an isolated/new-model database. An old Part schema is rejected without
/// mutation; use a fresh database path for the new model.
pub async fn open_session_kernel(url: &str) -> Result<KernelPorts> {
    Ok(open_kernel_runtime(url).await?.ports)
}

/// Infrastructure composition retains typed repositories; no raw connection
/// is handed to a session, provider, tool, or application caller.
pub struct SqliteKernel {
    pub database: Arc<sea_orm::DatabaseConnection>,
    pub ports: KernelPorts,
    pub workspaces: Arc<dyn agena_storage::WorkspaceRepository>,
    pub permissions: Arc<dyn agena_storage::PermissionRuleRepository>,
    pub model_catalog: Arc<dyn agena_storage::ModelCatalogRepository>,
    pub diagnostics: KernelDiagnostics,
    pub maintenance: port::KernelMaintenance,
}

pub async fn open_kernel_runtime(url: &str) -> Result<SqliteKernel> {
    let database = KernelDatabase::open(url).await.map_err(storage)?;
    let workspaces = Arc::new(crate::SeaWorkspaceRepository::new(database.writer.clone()));
    let permissions = Arc::new(crate::SeaPermissionRuleRepository::new(
        database.writer.clone(),
    ));
    let model_catalog = Arc::new(crate::SeaModelCatalogRepository::new(
        database.writer.clone(),
    ));
    let diagnostics = KernelDiagnostics::default();
    let repository_database = database.writer.clone();
    let (ports, maintenance) = compose_database(database, diagnostics.clone()).await?;
    Ok(SqliteKernel {
        database: repository_database,
        ports,
        workspaces,
        permissions,
        model_catalog,
        diagnostics,
        maintenance,
    })
}

#[cfg(test)]
async fn open_database(
    database: KernelDatabase,
    diagnostics: KernelDiagnostics,
) -> Result<KernelPorts> {
    Ok(compose_database(database, diagnostics).await?.0)
}

async fn compose_database(
    database: KernelDatabase,
    diagnostics: KernelDiagnostics,
) -> Result<(KernelPorts, port::KernelMaintenance)> {
    commands::recover(&database.writer).await?;
    let (sender, receiver) = mpsc::channel(1024);
    let epoch = KernelEpoch::new();
    let feed = Arc::new(ChangeFeed::with_budget(epoch, 256, diagnostics.2.clone()));
    let cache = diagnostics.1.clone();
    let views = diagnostics.2.clone();
    let live = Arc::new(content::LiveContent::new(diagnostics));
    let blob_pins = Arc::new(blobs::Pins::default());
    let history_pins = Arc::new(snapshots::Pins::default());
    let backend = Arc::new(Backend {
        reader: database.reader.clone(),
        sender,
        budget: Arc::new(Semaphore::new(MAX_PENDING_BYTES)),
        feed: feed.clone(),
        live: live.clone(),
        content_budget: Arc::new(Semaphore::new(MAX_CONTENT_PENDING_BYTES)),
        content_queue_slots: Arc::new(Semaphore::new(768)),
        content_reads: Arc::new(Semaphore::new(8)),
        // Content observers must not consume admission for status, cancellation
        // or permission lookups. Both lanes are bounded; the shared SQLite
        // reader pool still admits at most four actual SQL operations.
        control_reads: Arc::new(Semaphore::new(8)),
        frame_budget: Arc::new(Semaphore::new(content::events::FRAME_BUDGET_BYTES)),
        frame_reads: Arc::new(Semaphore::new(2)),
        blob_pins: blob_pins.clone(),
        blob_uploads: Arc::new(Semaphore::new(2)),
        cache: cache.clone(),
        views,
        history_pins: history_pins.clone(),
        snapshot_opens: Arc::new(Semaphore::new(8)),
        exports: archive::Exports::default(),
        imports: Arc::new(Semaphore::new(2)),
    });
    let maintenance = port::KernelMaintenance::new(backend.clone());
    tokio::spawn(writer(
        database,
        receiver,
        feed,
        live,
        blob_pins,
        cache,
        history_pins,
    ));
    Ok((port::session_kernel(backend, epoch), maintenance))
}

fn storage(error: impl std::fmt::Display) -> KernelError {
    KernelError::Storage(error.to_string())
}
fn invalid(error: impl std::fmt::Display) -> KernelError {
    KernelError::Invalid(error.to_string())
}
fn encode<T: serde::Serialize + ?Sized>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(storage)
}
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn sql_error(error: sqlx::Error) -> KernelError {
    if let sqlx::Error::Database(ref database) = error
        && database
            .code()
            .and_then(|code| code.parse::<u32>().ok())
            .is_some_and(|code| code & 0xff == 19)
    {
        return invalid(error);
    }
    storage(error)
}

fn rejected_commit(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().and_then(|code|code.parse::<u32>().ok()).is_some_and(|code| matches!(code & 0xff, 5 | 6 | 19)))
}

/// SQLx 0.8 decrements its depth only after SQLite accepts ROLLBACK. FULL and
/// some I/O errors can already have rolled back the SQLite transaction. In
/// that case a harmless empty BEGIN lets the driver's ROLLBACK clear its depth.
/// An existing transaction rejects this BEGIN and is rolled back normally.
/// This is only for paths on which COMMIT has never been attempted.
async fn abort_transaction(mut transaction: sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<()> {
    let _ = sqlx::query("BEGIN").execute(&mut *transaction).await;
    transaction.rollback().await.map_err(|error| {
        tracing::error!(%error,"cannot restore SQLite writer transaction state; reopen required");
        KernelError::Closed
    })
}

impl Backend {
    fn wrap_read<T>(&self, session: SessionId, value: T, bytes: usize) -> Result<ReadView<T>> {
        self.wrap_scoped(session, None, value, bytes)
    }
    fn wrap_scoped<T>(
        &self,
        session: SessionId,
        source: Option<ReadSource>,
        value: T,
        bytes: usize,
    ) -> Result<ReadView<T>> {
        let reserve = || {
            if let Some(source) = source {
                self.views.reserve_source(session, source, bytes)
            } else {
                self.views.reserve_view(session, bytes)
            }
        };
        let lease = reserve().or_else(|error| {
            if error != KernelError::Busy {
                return Err(error);
            }
            // Evict cache ownership before refusing a new consumer allocation.
            // Existing returned views retain their own shared lease.
            self.cache.evict();
            reserve()
        })?;
        Ok(ReadView::from_lease(value, lease))
    }

    async fn cached_read<T, F>(
        &self,
        admission: &Arc<Semaphore>,
        key: read_cache::Key,
        query: F,
    ) -> Result<ReadView<T>>
    where
        T: serde::Serialize + Send + Sync + 'static,
        F: std::future::Future<Output = Result<T>> + Send,
    {
        let fill = match self.cache.lookup(&key)? {
            read_cache::Lookup::Hit(view) => return Ok(view),
            read_cache::Lookup::Miss(fill) => fill,
        };
        let _read = admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let value = query.await?;
        let bytes = modeled_read_bytes(&value)?;
        let reserve = || self.views.reserve_owned(key.owner(), bytes);
        let lease = reserve().or_else(|error| {
            if error != KernelError::Busy {
                return Err(error);
            }
            self.cache.evict();
            reserve()
        })?;
        let view = ReadView::from_lease(value, lease);
        self.cache.install(key, fill, &view);
        Ok(view)
    }
}

#[async_trait]
impl KernelBackend for Backend {
    async fn prompt_window_status(
        &self,
        session: SessionId,
    ) -> Result<ReadView<PromptWindowStatus>> {
        lifecycle::prompt_window_status(self, session).await
    }

    async fn background_activities(
        &self,
        session: Option<SessionId>,
        after: Option<agena_domain::ActivityId>,
        active_only: bool,
        limit: usize,
    ) -> Result<ReadView<Vec<BackgroundActivityView>>> {
        lifecycle::background_activities(self, session, after, active_only, limit).await
    }
    async fn import_archive(
        &self,
        request: RequestId,
        workspace_id: i64,
        path: &std::path::Path,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        self.cache.ensure_open()?;
        let permit = self
            .imports
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let staged = import::prepare(path, request, workspace_id).await?;
        self.cache.ensure_open()?;
        let (response, receiver) = oneshot::channel();
        self.sender
            .send(Work::ImportArchive(Box::new(import::Job {
                staged,
                response,
                _permit: permit,
            })))
            .await
            .map_err(|_| KernelError::Closed)?;
        receiver.await.map_err(|_| KernelError::Closed)?
    }

    async fn history_after(
        &self,
        session: SessionId,
        after_entry: Option<EntryId>,
        limit: usize,
    ) -> Result<ReadView<HistoryPage>> {
        lifecycle::history_after(self, session, after_entry, limit).await
    }
    async fn find_task(
        &self,
        parent: SessionId,
        task_id: &str,
    ) -> Result<ReadView<Option<SessionId>>> {
        lifecycle::find_task(self, parent, task_id).await
    }

    async fn latest_round(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        self.cache.ensure_open()?;
        let _read = self
            .control_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let id:Option<i64>=sqlx::query_scalar("SELECT id FROM agena_rounds WHERE session_id=? AND phase!='running' AND provider_state_json IS NOT NULL ORDER BY id DESC LIMIT 1").bind(session.get()).fetch_optional(self.reader.get_sqlite_connection_pool()).await.map_err(sql_error)?;
        let value = if let Some(id) = id {
            Some(
                reads::round(
                    self.reader.get_sqlite_connection_pool(),
                    session,
                    RoundId::try_from(id).map_err(storage)?,
                    self.live.diagnostics(),
                )
                .await?,
            )
        } else {
            let mut conn = self
                .reader
                .get_sqlite_connection_pool()
                .acquire()
                .await
                .map_err(storage)?;
            reads::metadata(&mut conn, session).await?;
            None
        };
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(session, value, bytes)
    }
    async fn latest_usage_round(
        &self,
        session: SessionId,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        self.cache.ensure_open()?;
        let _read = self
            .control_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM agena_rounds WHERE session_id=? AND phase!='running' AND json_extract(provider_state_json,'$.usage') IS NOT NULL ORDER BY id DESC LIMIT 1",
        )
        .bind(session.get())
        .fetch_optional(self.reader.get_sqlite_connection_pool())
        .await
        .map_err(sql_error)?;
        let value = if let Some(id) = id {
            Some(
                reads::round(
                    self.reader.get_sqlite_connection_pool(),
                    session,
                    RoundId::try_from(id).map_err(storage)?,
                    self.live.diagnostics(),
                )
                .await?,
            )
        } else {
            let mut conn = self
                .reader
                .get_sqlite_connection_pool()
                .acquire()
                .await
                .map_err(storage)?;
            reads::metadata(&mut conn, session).await?;
            None
        };
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(session, value, bytes)
    }
    async fn configuration(&self, session: SessionId) -> Result<ReadView<SessionConfiguration>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Configuration(session),
            async {
                let mut tx = self
                    .reader
                    .get_sqlite_connection_pool()
                    .begin()
                    .await
                    .map_err(storage)?;
                lifecycle::configuration(&mut tx, session).await
            },
        )
        .await
    }
    async fn query_summaries(&self, query: SummaryQuery) -> Result<ReadView<SessionSummaryPage>> {
        lifecycle::query_summaries(self, query).await
    }
    async fn workspace_statistics(
        &self,
        ids: &[i64],
    ) -> Result<ReadView<std::collections::BTreeMap<i64, WorkspaceStatistics>>> {
        lifecycle::workspace_statistics(self, ids).await
    }
    async fn rounds(
        &self,
        session: SessionId,
        after: Option<RoundId>,
        limit: usize,
    ) -> Result<ReadView<Vec<ProviderRoundView>>> {
        lifecycle::rounds(self, session, after, limit).await
    }
    async fn pending_deliveries(
        &self,
        session: SessionId,
        after: Option<agena_domain::CommandId>,
        limit: usize,
    ) -> Result<ReadView<Vec<DeliveryView>>> {
        lifecycle::pending_deliveries(self, session, after, limit).await
    }
    fn watch_all(&self, after: Option<WatchCursor>) -> Result<KernelWatch> {
        self.cache.ensure_open()?;
        Ok(self
            .feed
            .watch_all(after)
            .with_lease(self.views.subscribe_global()?))
    }
    async fn export_jsonl(
        &self,
        lease: port::HistorySnapshotLease,
        info: &ReadView<HistorySnapshotInfo>,
        target: &mut (dyn tokio::io::AsyncWrite + Unpin + Send),
    ) -> Result<ExportReport> {
        archive::export(self, lease, info, target).await
    }
    async fn open_history_snapshot(
        &self,
        session: SessionId,
        epoch: KernelEpoch,
    ) -> Result<port::HistorySnapshotOpened> {
        self.cache.ensure_open()?;
        let _admission = self
            .snapshot_opens
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let (response, receive) = oneshot::channel();
        self.sender
            .try_send(Work::OpenHistorySnapshot {
                session,
                epoch,
                response,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => KernelError::Busy,
                mpsc::error::TrySendError::Closed(_) => KernelError::Closed,
            })?;
        receive.await.map_err(|_| KernelError::Closed)?
    }
    fn retain_history_snapshot(&self, lease: port::HistorySnapshotLease) -> Result<()> {
        self.history_pins.retain(lease)
    }
    fn release_history_snapshot(&self, lease: port::HistorySnapshotLease) {
        self.history_pins.release(lease);
    }
    async fn collect_history_snapshots(&self) -> Result<port::HistorySnapshotGc> {
        self.cache.ensure_open()?;
        let (response, receive) = oneshot::channel();
        self.sender
            .try_send(Work::CollectHistorySnapshots(response))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => KernelError::Busy,
                mpsc::error::TrySendError::Closed(_) => KernelError::Closed,
            })?;
        receive.await.map_err(|_| KernelError::Closed)?
    }
    async fn snapshot_page(
        &self,
        lease: port::HistorySnapshotLease,
        after: Option<SnapshotCursor>,
        limit: usize,
    ) -> Result<ReadView<SnapshotPage>> {
        self.cache.ensure_open()?;
        let _read = self
            .content_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let value = snapshots::page(
            self.reader.get_sqlite_connection_pool(),
            lease,
            after,
            limit,
            self.live.diagnostics(),
        )
        .await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(lease.session, value, bytes)
    }
    async fn prompt(&self, session: SessionId, round: RoundId) -> Result<ReadView<PromptView>> {
        self.cached_read(
            &self.content_reads,
            read_cache::Key::Prompt(session, round),
            prompt::read(
                self.reader.get_sqlite_connection_pool(),
                session,
                round,
                self.live.diagnostics(),
            ),
        )
        .await
    }
    async fn text_range_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
        after: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>> {
        self.cache.ensure_open()?;
        let value = content::range_at(self, session, boundary, after, limit).await?;
        let bytes = std::mem::size_of::<TextRange>()
            + 512
            + value.bytes.capacity()
            + value
                .stream
                .capture_error
                .as_ref()
                .map_or(0, String::capacity);
        self.wrap_scoped(
            session,
            Some(ReadSource::Content(boundary.id)),
            value,
            bytes,
        )
    }
    async fn event_page_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<ReadView<EventPage>> {
        self.cache.ensure_open()?;
        let value = content::events::page_at(self, session, boundary, after, limit).await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_scoped(
            session,
            Some(ReadSource::Content(boundary.id)),
            value,
            bytes,
        )
    }
    async fn event_snapshot_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
    ) -> Result<ReadView<EventSnapshot>> {
        self.cache.ensure_open()?;
        let value = content::events::snapshot_at(self, session, boundary).await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_scoped(
            session,
            Some(ReadSource::Content(boundary.id)),
            value,
            bytes,
        )
    }
    async fn summaries(
        &self,
        workspace_id: i64,
        before: Option<SessionSummaryCursor>,
        limit: usize,
    ) -> Result<ReadView<SessionSummaryPage>> {
        let limit = limit.clamp(1, 200);
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Summaries(workspace_id, before, limit),
            reads::summaries(
                self.reader.get_sqlite_connection_pool(),
                workspace_id,
                before,
                limit,
                self.live.diagnostics(),
            ),
        )
        .await
    }
    async fn control_snapshot(
        &self,
        session: SessionId,
        limit: usize,
    ) -> Result<ReadView<SessionControlSnapshot>> {
        control_snapshot::read(self, session, limit).await
    }
    async fn summary(&self, session: SessionId) -> Result<ReadView<SessionSummaryView>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Summary(session),
            async {
                let mut connection = self
                    .reader
                    .get_sqlite_connection_pool()
                    .acquire()
                    .await
                    .map_err(storage)?;
                metrics::add(&self.live.diagnostics().0.summary_queries, 1);
                reads::summary(&mut connection, session).await
            },
        )
        .await
    }
    async fn pending_interactions(
        &self,
        session: SessionId,
        after: Option<PendingInteractionCursor>,
        limit: usize,
    ) -> Result<ReadView<PendingInteractionPage>> {
        let limit = limit.clamp(1, 200);
        self.cached_read(
            &self.control_reads,
            read_cache::Key::PendingInteractions(session, after, limit),
            reads::pending_interactions(
                self.reader.get_sqlite_connection_pool(),
                session,
                after,
                limit,
                self.live.diagnostics(),
            ),
        )
        .await
    }
    async fn entry(&self, session: SessionId, entry: EntryId) -> Result<ReadView<EntryView>> {
        self.cached_read(
            &self.content_reads,
            read_cache::Key::Entry(session, entry),
            reads::entry(
                self.reader.get_sqlite_connection_pool(),
                session,
                entry,
                self.live.diagnostics(),
            ),
        )
        .await
    }
    async fn owned_tool_entries(
        &self,
        session: SessionId,
        after: Option<OwnedToolCursor>,
        limit: usize,
    ) -> Result<ReadView<OwnedToolPage>> {
        let limit = limit.clamp(1, 128);
        self.cached_read(
            &self.content_reads,
            read_cache::Key::OwnedTools(session, after, limit),
            owned_tools::read(
                self.reader.get_sqlite_connection_pool(),
                session,
                after,
                limit,
                self.live.diagnostics(),
            ),
        )
        .await
    }
    async fn active_executions(
        &self,
        session: SessionId,
        after: Option<ActiveExecutionCursor>,
        limit: usize,
    ) -> Result<ReadView<ActiveExecutionPage>> {
        let limit = limit.clamp(1, 200);
        self.cached_read(
            &self.control_reads,
            read_cache::Key::ActiveExecutions(session, after, limit),
            reads::active_executions(
                self.reader.get_sqlite_connection_pool(),
                session,
                after,
                limit,
                self.live.diagnostics(),
            ),
        )
        .await
    }
    fn invalidate_read_cache(&self) {
        self.cache.clear();
    }
    fn admit_blob_upload(&self) -> Result<OwnedSemaphorePermit> {
        self.blob_uploads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)
    }
    fn retain_blob(&self, lease: port::BlobLease) -> Result<()> {
        self.blob_pins.retain(lease)
    }
    fn release_blob(&self, lease: port::BlobLease) {
        self.blob_pins.release(lease);
    }
    async fn collect_blobs(&self) -> Result<port::BlobGc> {
        let (response, receive) = oneshot::channel();
        self.sender
            .send(Work::CollectBlobs(response))
            .await
            .map_err(|_| KernelError::Closed)?;
        receive.await.map_err(|_| KernelError::Closed)?
    }
    async fn blob_range(
        &self,
        lease: port::BlobLease,
        offset: u64,
        limit: usize,
    ) -> Result<ReadView<BlobRange>> {
        self.cache.ensure_open()?;
        let value = blobs::range(self, lease, offset, limit).await?;
        let bytes = value.bytes.capacity() + std::mem::size_of::<BlobRange>();
        self.wrap_scoped(
            lease.session,
            Some(ReadSource::Blob(lease.id)),
            value,
            bytes,
        )
    }
    async fn entry_blobs(
        &self,
        session: SessionId,
        entry: EntryId,
    ) -> Result<ReadView<Vec<BlobDescriptor>>> {
        self.cache.ensure_open()?;
        let value = blobs::entry_blobs(self, session, entry).await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(session, value, bytes)
    }
    async fn append_event(
        &self,
        scope: port::OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        timestamp_ms: Option<i64>,
        input: port::EventInput,
    ) -> Result<EventCursor> {
        content::events::admit(self, scope, content, epoch, timestamp_ms, input).await
    }
    fn try_append_event(
        &self,
        scope: port::OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        timestamp_ms: Option<i64>,
        input: port::EventInput,
    ) -> Result<EventCursor> {
        content::events::try_admit(self, scope, content, epoch, timestamp_ms, input)
    }
    fn record_capture_loss(
        &self,
        scope: port::OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        bytes: u64,
        diagnostic: &str,
    ) -> Result<()> {
        content::loss::record(self, scope, content, epoch, bytes, diagnostic)
    }
    async fn capture_losses(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<CaptureLossCursor>,
        limit: usize,
    ) -> Result<ReadView<CaptureLossPage>> {
        self.cache.ensure_open()?;
        let value = content::loss::page(self, session, content, after, limit).await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_scoped(session, Some(ReadSource::Content(content)), value, bytes)
    }
    async fn event_page(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<ReadView<EventPage>> {
        self.cache.ensure_open()?;
        let value = content::events::page(self, session, content, after, limit).await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_scoped(session, Some(ReadSource::Content(content)), value, bytes)
    }
    async fn event_snapshot(
        &self,
        session: SessionId,
        content: ContentId,
    ) -> Result<ReadView<EventSnapshot>> {
        self.cache.ensure_open()?;
        let value = content::events::snapshot(self, session, content).await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_scoped(session, Some(ReadSource::Content(content)), value, bytes)
    }
    fn retain_output(&self, scope: port::OutputScope, stream: &ContentStreamView) -> Result<()> {
        self.live.retain(scope, stream)
    }
    fn release_output(&self, scope: port::OutputScope, content: ContentId) {
        self.live.release(scope, content);
    }
    async fn append_text(
        &self,
        scope: port::OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        text: &str,
    ) -> Result<TextCursor> {
        if text.len() > content::MAX_APPEND_BYTES {
            return Err(invalid(
                "text append exceeds 256 KiB; stream bounded chunks",
            ));
        }
        let charge = text.len() + 512;
        let permit = self
            .content_budget
            .clone()
            .try_acquire_many_owned(charge as u32)
            .map_err(|_| KernelError::Busy)?;
        let slot = self
            .content_queue_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let (response, receive) = oneshot::channel();
        let append = content::Append {
            scope,
            content,
            epoch,
            text: text.into(),
            response,
            permit,
            _queue_slot: slot,
        };
        self.sender
            .try_send(Work::Append(Box::new(append)))
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => KernelError::Busy,
                mpsc::error::TrySendError::Closed(_) => KernelError::Closed,
            })?;
        receive.await.map_err(|_| KernelError::Closed)?
    }
    async fn text_range(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>> {
        self.cache.ensure_open()?;
        let value = content::range(self, session, content, after, limit).await?;
        let bytes = value.bytes.capacity()
            + std::mem::size_of::<TextRange>()
            + value
                .stream
                .capture_error
                .as_ref()
                .map_or(0, String::capacity);
        self.wrap_scoped(session, Some(ReadSource::Content(content)), value, bytes)
    }
    fn try_append_text(
        &self,
        scope: port::OutputScope,
        content: ContentId,
        epoch: ContentEpoch,
        text: &str,
    ) -> Result<TextCursor> {
        self.cache.ensure_open()?;
        if text.len() > content::MAX_APPEND_BYTES {
            return Err(invalid(
                "text append exceeds 256 KiB; stream bounded chunks",
            ));
        }
        let permit = self
            .content_budget
            .clone()
            .try_acquire_many_owned((text.len() + 512) as u32)
            .map_err(|_| KernelError::Busy)?;
        let slot = self
            .content_queue_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let (response, mut receive) = oneshot::channel();
        let append = content::Append {
            scope,
            content,
            epoch,
            text: text.into(),
            response,
            permit,
            _queue_slot: slot,
        };
        if self.live.try_append(append)?.is_some() {
            return Err(KernelError::Conflict(
                "content capture is closed or its lease was revoked".into(),
            ));
        }
        receive.try_recv().map_err(|_| KernelError::Closed)?
    }
    async fn activity(
        &self,
        session: SessionId,
        id: agena_domain::ActivityId,
    ) -> Result<ReadView<BackgroundActivityView>> {
        activity::read(self, session, id).await
    }
    async fn active_activities(
        &self,
        session: SessionId,
        after: Option<ActiveActivityCursor>,
        limit: usize,
    ) -> Result<ReadView<ActiveActivityPage>> {
        activity::active(self, session, after, limit).await
    }
    async fn watch_content(&self, session: SessionId, content: ContentId) -> Result<ContentWatch> {
        self.cache.ensure_open()?;
        let subscription = self.views.subscribe_content(session, content)?;
        let (response, receive) = oneshot::channel();
        self.sender
            .try_send(Work::WatchContent {
                session,
                content,
                response,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => KernelError::Busy,
                mpsc::error::TrySendError::Closed(_) => KernelError::Closed,
            })?;
        receive
            .await
            .map_err(|_| KernelError::Closed)?
            .map(|watch| watch.with_lease(subscription))
    }
    async fn tool(&self, session: SessionId, tool: ToolId) -> Result<ReadView<ToolView>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Tool(session, tool),
            async {
                reads::tool(
                    self.reader.get_sqlite_connection_pool(),
                    session,
                    tool,
                    self.live.diagnostics(),
                )
                .await
            },
        )
        .await
    }
    async fn interaction(
        &self,
        session: SessionId,
        interaction: InteractionId,
    ) -> Result<ReadView<InteractionView>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Interaction(session, interaction),
            async {
                reads::interaction(
                    self.reader.get_sqlite_connection_pool(),
                    session,
                    interaction,
                    self.live.diagnostics(),
                )
                .await
            },
        )
        .await
    }
    async fn round(
        &self,
        session: SessionId,
        round: RoundId,
    ) -> Result<ReadView<ProviderRoundView>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Round(session, round),
            async {
                reads::round(
                    self.reader.get_sqlite_connection_pool(),
                    session,
                    round,
                    self.live.diagnostics(),
                )
                .await
            },
        )
        .await
    }
    async fn failed_round(
        &self,
        session: SessionId,
        execution: ExecutionId,
        generation: AttemptGeneration,
    ) -> Result<ReadView<Option<ProviderRoundView>>> {
        let value = reads::failed_round(
            self.reader.get_sqlite_connection_pool(),
            session,
            execution,
            generation,
            self.live.diagnostics(),
        )
        .await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(session, value, bytes)
    }
    fn watch(&self, session: SessionId, after: Option<WatchCursor>) -> Result<KernelWatch> {
        self.cache.ensure_open()?;
        let lease = self.views.subscribe(session)?;
        Ok(self.feed.watch(session, after).with_lease(lease))
    }
    async fn shutdown(&self) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Work::Shutdown(sender))
            .await
            .map_err(|_| KernelError::Closed)?;
        receiver.await.map_err(|_| KernelError::Closed)?
    }
    async fn commit(&self, request: RequestId, command: Command) -> Reply {
        let _retention_read = if matches!(command, Command::PruneContent { .. }) {
            Some(
                self.frame_reads
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| KernelError::Busy)?,
            )
        } else {
            None
        };
        let encoded = blobs::command_bytes(&command)?;
        let bytes = encoded.len() + 512;
        if bytes > MAX_COMMAND_BYTES {
            return Err(invalid(
                "command exceeds 1 MiB; large output needs a content stream",
            ));
        }
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(bytes as u32)
            .map_err(|_| KernelError::Busy)?;
        let (response, receive) = oneshot::channel();
        let pending = Pending {
            request,
            command,
            digest: Sha256::digest(&encoded).to_vec(),
            bytes,
            response,
            _permit: permit,
            _retention_read,
        };
        drop(encoded);
        self.sender
            .try_send(Work::Command(Box::new(pending)))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => KernelError::Busy,
                mpsc::error::TrySendError::Closed(_) => KernelError::Closed,
            })?;
        receive.await.map_err(|_| KernelError::CommitUnknown {
            request_id: request,
            diagnostic: "writer response disconnected".into(),
        })?
    }

    async fn metadata(&self, session: SessionId) -> Result<ReadView<SessionMetadata>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Metadata(session),
            async {
                let mut connection = self
                    .reader
                    .get_sqlite_connection_pool()
                    .acquire()
                    .await
                    .map_err(storage)?;
                metrics::add(&self.live.diagnostics().0.metadata_queries, 1);
                reads::metadata(&mut connection, session).await
            },
        )
        .await
    }
    async fn control_version(&self, session: SessionId) -> Result<ReadView<i64>> {
        self.cache.ensure_open()?;
        let _read = self
            .control_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let version = sqlx::query_scalar(
            "SELECT control_version FROM agena_sessions WHERE id=? AND deleted_at_ms IS NULL",
        )
        .bind(session.get())
        .fetch_optional(self.reader.get_sqlite_connection_pool())
        .await
        .map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("session".into()))?;
        self.views.wrap(session, version)
    }
    async fn execution(
        &self,
        session: SessionId,
        execution: ExecutionId,
    ) -> Result<ReadView<ExecutionView>> {
        self.cached_read(
            &self.control_reads,
            read_cache::Key::Execution(session, execution),
            async {
                let mut connection = self
                    .reader
                    .get_sqlite_connection_pool()
                    .acquire()
                    .await
                    .map_err(storage)?;
                metrics::add(&self.live.diagnostics().0.detail_queries, 1);
                reads::execution_visible(&mut connection, session, execution).await
            },
        )
        .await
    }
    async fn history_window(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        message_ids: &[i64],
        limit: usize,
    ) -> Result<ReadView<HistoryPage>> {
        let _permit = self
            .content_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let value = history_views::history_window(
            self.reader.get_sqlite_connection_pool(),
            session,
            before,
            message_ids,
            limit,
            self.live.diagnostics(),
        )
        .await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(session, value, bytes)
    }
    async fn message_history(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        message_limit: usize,
        entry_limit: usize,
    ) -> Result<ReadView<MessageHistoryPage>> {
        let _permit = self
            .content_reads
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::Busy)?;
        let value = history_views::message_history(
            self.reader.get_sqlite_connection_pool(),
            session,
            before,
            message_limit,
            entry_limit,
            self.live.diagnostics(),
        )
        .await?;
        let bytes = modeled_read_bytes(&value)?;
        self.wrap_read(session, value, bytes)
    }
    async fn history(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<ReadView<HistoryPage>> {
        let limit = limit.clamp(1, 200);
        self.cached_read(
            &self.content_reads,
            read_cache::Key::History(session, before, limit),
            reads::history(
                self.reader.get_sqlite_connection_pool(),
                session,
                before,
                limit,
                self.live.diagnostics(),
            ),
        )
        .await
    }
}

async fn writer(
    database: KernelDatabase,
    mut receiver: mpsc::Receiver<Work>,
    feed: Arc<ChangeFeed>,
    live: Arc<content::LiveContent>,
    blob_pins: Arc<blobs::Pins>,
    cache: Arc<read_cache::ReadCache>,
    history_pins: Arc<snapshots::Pins>,
) {
    let mut shutdown = Vec::new();
    let mut carry = None;
    let mut fatal = None;
    let mut retry_after = tokio::time::Instant::now();
    loop {
        let work = if let Some(work) = carry.take() {
            work
        } else {
            tokio::select! {
                work = receiver.recv() => match work { Some(work) => work, None => break },
                _ = live.admission_changed(), if fatal.is_none() => { continue; }
                _ = tokio::time::sleep_until(live.next_checkpoint().max(retry_after)), if fatal.is_none() => {
                    if tokio::time::Instant::now() >= retry_after {
                        match content::checkpoint(&database,&live,true,false).await {
                            // The next deadline remains due while older sources
                            // await their bounded turn. New tails have their own
                            // deadline and cannot be sealed by this sweep.
                            Ok(())=>{}
                            Err(error)=>{
                                tracing::warn!(%error, "content checkpoint failed; accepted tail retained");
                                retry_after = tokio::time::Instant::now()+Duration::from_millis(500);
                                if error.requires_reopen() {
                                    fatal = Some(storage(&error));
                                    receiver.close();
                                    cache.close();
                                    live.fail_closed(&error.to_string());
                                }
                            }
                        }
                        if fatal.is_none() && history_pins.needs_collection()
                            && let Err(error) = snapshots::collect(&database, &history_pins).await {
                            tracing::warn!(%error, "history snapshot collection failed");
                            if matches!(error, KernelError::CommitUnknown {..} | KernelError::Closed) {
                                fatal = Some(storage(&error));
                                receiver.close(); cache.close(); live.fail_closed(&error.to_string());
                            }
                        }
                    }
                    continue;
                }
            }
        };
        if fatal.is_some() {
            match work {
                Work::ImportArchive(job) => {
                    let _ = job.response.send(Err(KernelError::Closed));
                }
                Work::OpenHistorySnapshot { response, .. } => {
                    let _ = response.send(Err(KernelError::Closed));
                }
                Work::CollectHistorySnapshots(response) => {
                    let _ = response.send(Err(KernelError::Closed));
                }
                Work::Command(pending) => {
                    let _ = pending.response.send(Err(KernelError::Closed));
                }
                Work::Append(append) => {
                    let _ = append.response.send(Err(KernelError::Closed));
                }
                Work::AppendEvent(append) => {
                    let _ = append.response.send(Err(KernelError::Closed));
                }
                Work::WatchContent { response, .. } => {
                    let _ = response.send(Err(KernelError::Closed));
                }
                Work::Shutdown(sender) => shutdown.push(sender),
                Work::CollectBlobs(response) => {
                    let _ = response.send(Err(KernelError::Closed));
                }
            }
            continue;
        }
        let first = match work {
            Work::ImportArchive(job) => {
                let result = import::publish(&database, &job.staged, &feed, &cache).await;
                if matches!(
                    result,
                    Err(KernelError::CommitUnknown { .. } | KernelError::Closed)
                ) {
                    fatal = Some(storage("archive import requires reopening the writer"));
                    receiver.close();
                    cache.close();
                    live.fail_closed("archive import commit outcome is unknown");
                }
                let _ = job.response.send(result);
                continue;
            }
            Work::OpenHistorySnapshot {
                session,
                epoch,
                response,
            } => {
                let result = async {
                    snapshots::collect(&database, &history_pins).await?;
                    snapshots::open(
                        &database,
                        session,
                        epoch,
                        &history_pins,
                        &live.diagnostics().2,
                        &cache,
                    )
                    .await
                }
                .await;
                if matches!(
                    result,
                    Err(KernelError::CommitUnknown { .. } | KernelError::Closed)
                ) {
                    fatal = Some(storage("history snapshot commit outcome is unknown"));
                    receiver.close();
                    cache.close();
                    live.fail_closed("history snapshot commit outcome is unknown");
                }
                let _ = response.send(result);
                continue;
            }
            Work::CollectHistorySnapshots(response) => {
                let result = snapshots::collect(&database, &history_pins).await;
                if matches!(
                    result,
                    Err(KernelError::CommitUnknown { .. } | KernelError::Closed)
                ) {
                    fatal = Some(storage("history snapshot collection outcome is unknown"));
                    receiver.close();
                    cache.close();
                    live.fail_closed("history snapshot collection outcome is unknown");
                }
                let _ = response.send(result);
                continue;
            }
            Work::CollectBlobs(response) => {
                let result = blobs::collect(&database, &blob_pins).await;
                if matches!(
                    result,
                    Err(KernelError::CommitUnknown { .. } | KernelError::Closed)
                ) {
                    fatal = Some(storage("blob collection requires reopening the writer"));
                    receiver.close();
                    cache.close();
                    live.fail_closed("blob collection commit outcome is unknown");
                }
                let _ = response.send(result);
                continue;
            }
            Work::Command(first) => *first,
            Work::Append(append) => {
                if let Some(append) = live.append(*append) {
                    let result = async {
                        let mut connection = database
                            .writer
                            .get_sqlite_connection_pool()
                            .acquire()
                            .await
                            .map_err(storage)?;
                        activity::check_output(&mut connection, append.scope).await?;
                        Err(KernelError::Conflict("content capture is closed".into()))
                    }
                    .await;
                    let _ = append.response.send(result);
                }
                if tokio::time::Instant::now() >= retry_after
                    && let Err(error) = content::checkpoint(&database, &live, false, false).await
                {
                    tracing::warn!(%error, "content checkpoint failed; accepted tail retained");
                    retry_after = tokio::time::Instant::now() + Duration::from_millis(500);
                    if error.requires_reopen() {
                        fatal = Some(storage(&error));
                        receiver.close();
                        cache.close();
                        live.fail_closed(&error.to_string());
                    }
                }
                continue;
            }
            Work::AppendEvent(append) => {
                if let Some(append) = live.append_event(*append) {
                    let result = async {
                        let mut connection = database
                            .writer
                            .get_sqlite_connection_pool()
                            .acquire()
                            .await
                            .map_err(storage)?;
                        activity::check_output(&mut connection, append.scope).await?;
                        Err(KernelError::Conflict("content capture is closed".into()))
                    }
                    .await;
                    let _ = append.response.send(result);
                }
                if tokio::time::Instant::now() >= retry_after
                    && let Err(error) = content::checkpoint(&database, &live, false, false).await
                {
                    tracing::warn!(%error,"event checkpoint failed; accepted content retained");
                    retry_after = tokio::time::Instant::now() + Duration::from_millis(500);
                    if error.requires_reopen() {
                        fatal = Some(storage(&error));
                        receiver.close();
                        cache.close();
                        live.fail_closed(&error.to_string());
                    }
                }
                continue;
            }
            Work::WatchContent {
                session,
                content: id,
                response,
            } => {
                let _ = response.send(content::watch(&database.reader, &live, session, id).await);
                continue;
            }
            Work::Shutdown(sender) => {
                shutdown.push(sender);
                receiver.close();
                continue;
            }
        };
        let mut bytes = first.bytes;
        let mut batch = vec![first];
        // A short bounded collection window permits real group commit while
        // keeping send/cancel/interaction latency independent of providers.
        tokio::time::sleep(Duration::from_millis(1)).await;
        while batch.len() < MAX_BATCH_COMMANDS && bytes < MAX_BATCH_BYTES {
            let Ok(next) = receiver.try_recv() else {
                break;
            };
            let next = match next {
                Work::Command(next) => *next,
                other => {
                    // Never batch across an append or another FIFO boundary.
                    carry = Some(other);
                    break;
                }
            };
            bytes += next.bytes;
            batch.push(next);
        }
        let results = commit_batch(&database, &batch, &feed, &live, &blob_pins, &cache).await;
        if results.iter().any(|result| {
            matches!(
                result,
                Err(KernelError::CommitUnknown { .. } | KernelError::Closed)
            )
        }) {
            fatal = Some(storage("command commit outcome is unknown"));
            receiver.close();
            cache.close();
            live.fail_closed("command commit outcome is unknown");
        }
        for (pending, result) in batch.into_iter().zip(results) {
            #[cfg(test)]
            crash_window_tests::hit("command.before_receipt", Some(pending.request));
            #[cfg(test)]
            let request = pending.request;
            let _ = pending.response.send(result);
            #[cfg(test)]
            crash_window_tests::hit("command.after_receipt", Some(request));
        }
    }
    let checkpoint = if let Some(error) = fatal {
        Err(error)
    } else {
        content::checkpoint(&database, &live, true, true)
            .await
            .map_err(storage)
    };
    if let Err(error) = &checkpoint {
        cache.close();
        live.fail_closed(&error.to_string());
    }
    let result = checkpoint.and(database.close().await.map_err(storage));
    #[cfg(test)]
    crash_window_tests::writer_closed(&cache);
    cache.close();
    feed.close();
    if let Err(error) = &result {
        tracing::warn!(%error, "session kernel close failed");
    }
    for sender in shutdown {
        let _ = sender.send(result.clone());
    }
}

async fn commit_batch(
    database: &KernelDatabase,
    batch: &[Pending],
    feed: &ChangeFeed,
    live: &content::LiveContent,
    blob_pins: &blobs::Pins,
    cache: &Arc<read_cache::ReadCache>,
) -> Vec<Reply> {
    let _gates = live.admission_gates();
    let mut connection = match database.writer.get_sqlite_connection_pool().acquire().await {
        Ok(connection) => connection,
        Err(error) => return vec![Err(storage(error)); batch.len()],
    };
    // SQLx Transaction rolls back on drop, including cancellation of this
    // worker. Its executor is never held across a tool or network operation.
    let transaction = connection.begin_with("BEGIN IMMEDIATE").await;
    let mut transaction = match transaction {
        Ok(transaction) => transaction,
        Err(error) => return vec![Err(storage(error)); batch.len()],
    };
    let sequence: i64 = match sqlx::query_scalar("UPDATE agena_commits SET sequence=sequence+1 WHERE id=1 AND sequence<9223372036854775807 RETURNING sequence")
        .fetch_one(&mut *transaction).await {
        Ok(sequence) => sequence,
        Err(error) => {
            let result = abort_transaction(transaction).await.and(Err(storage(error)));
            return vec![result; batch.len()];
        },
    };
    let sequence = match CommitSequence::try_from(sequence) {
        Ok(value) => value,
        Err(error) => {
            let result = abort_transaction(transaction)
                .await
                .and(Err(storage(error)));
            return vec![result; batch.len()];
        }
    };
    let mut results = Vec::with_capacity(batch.len());
    let mut changes = Vec::with_capacity(batch.len());
    for (ordinal, pending) in batch.iter().enumerate() {
        if let Err(error) = sqlx::query("SAVEPOINT kernel_command")
            .execute(&mut *transaction)
            .await
        {
            let error = abort_transaction(transaction)
                .await
                .err()
                .unwrap_or_else(|| storage(error));
            return aborted_batch(results, batch.len(), sequence, error);
        }
        let position = CommitPosition {
            sequence,
            ordinal: ordinal as u32,
        };
        let result = execute(&mut transaction, pending, position, live).await;
        let statements: &[&str] = if result.is_ok() {
            &["RELEASE kernel_command"]
        } else {
            &["ROLLBACK TO kernel_command", "RELEASE kernel_command"]
        };
        for statement in statements {
            if let Err(error) = sqlx::query(statement).execute(&mut *transaction).await {
                // No COMMIT has been attempted; even an implicit whole-tx
                // rollback cannot turn these commands into successful facts.
                let error = abort_transaction(transaction)
                    .await
                    .err()
                    .unwrap_or_else(|| storage(error));
                return aborted_batch(results, batch.len(), sequence, error);
            }
        }
        match result {
            Ok((receipt, (updates, entry_changes))) => {
                results.push(Ok(receipt));
                changes.push((updates, entry_changes));
            }
            Err(error) => {
                live.release_gate(pending.request);
                results.push(Err(error));
                changes.push((Vec::new(), EntryChanges::default()));
            }
        }
    }
    let mut affected = results
        .iter()
        .zip(batch)
        .filter_map(|(result, pending)| {
            result
                .as_ref()
                .ok()
                .filter(|receipt| receipt.position.sequence == sequence)
                .filter(|_| !matches!(pending.command, Command::PinSnapshotBlob { .. }))
                .map(|receipt| change_target(&pending.command, &receipt.value).0)
        })
        .collect::<Vec<_>>();
    for (_, change) in &changes {
        if let Some(list) = &change.list_change {
            for parent in &list.parents {
                if !affected.contains(&parent.session) {
                    affected.push(parent.session);
                }
            }
        }
    }
    // Capture ownership from this transaction, before COMMIT. A summary-page
    // cache belongs to its workspace, including an empty page cached before
    // the workspace's first session is created.
    let workspaces = if affected.is_empty() {
        Vec::new()
    } else {
        let identities = match encode(&affected) {
            Ok(value) => value,
            Err(error) => {
                let error = abort_transaction(transaction).await.err().unwrap_or(error);
                return aborted_batch(results, batch.len(), sequence, error);
            }
        };
        match sqlx::query_scalar::<_, i64>("SELECT DISTINCT workspace_id FROM agena_sessions WHERE id IN (SELECT value FROM json_each(?))")
            .bind(identities).fetch_all(&mut *transaction).await {
            Ok(workspaces) => workspaces,
            Err(error) => {
                let error = abort_transaction(transaction).await.err().unwrap_or_else(|| sql_error(error));
                return aborted_batch(results, batch.len(), sequence, error);
            }
        }
    };
    let shared_entries = changes
        .iter()
        .flat_map(|(_, change)| change.entries.iter().copied())
        .collect::<Vec<_>>();
    let shared_sessions = batch
        .iter()
        .zip(&results)
        .zip(&changes)
        .filter_map(|((pending, result), (_, change))| {
            let receipt = result.as_ref().ok()?;
            if receipt.position.sequence != sequence || !change.requires_snapshot {
                return None;
            }
            if matches!(pending.command, Command::DeleteSession { .. })
                || matches!(receipt.value, Mutation::Execution(_))
            {
                Some(change_target(&pending.command, &receipt.value).0)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let shared_family = if shared_entries.is_empty() && shared_sessions.is_empty() {
        false
    } else {
        let identities = match encode(&shared_entries) {
            Ok(ids) => ids,
            Err(error) => {
                let error = abort_transaction(transaction).await.err().unwrap_or(error);
                return aborted_batch(results, batch.len(), sequence, error);
            }
        };
        let origins = match encode(&shared_sessions) {
            Ok(ids) => ids,
            Err(error) => {
                let error = abort_transaction(transaction).await.err().unwrap_or(error);
                return aborted_batch(results, batch.len(), sequence, error);
            }
        };
        match sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM agena_entries e JOIN agena_segment_entries m ON m.entry_id=e.id JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE (e.id IN (SELECT value FROM json_each(?)) OR e.origin_session_id IN (SELECT value FROM json_each(?))) AND s.session_id!=e.origin_session_id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal AND owner.deleted_at_ms IS NULL)").bind(identities).bind(origins).fetch_one(&mut *transaction).await {
            Ok(shared)=>shared,
            Err(error)=>{let error=abort_transaction(transaction).await.err().unwrap_or_else(||sql_error(error));return aborted_batch(results,batch.len(),sequence,error);}
        }
    };
    // A later command in this batch may already have sealed an earlier open.
    // Receipts remain exact per-command observations, while installation uses
    // the final transaction state and can never reopen that source in memory.
    let mut opened_heads = std::collections::HashMap::new();
    for (pending, result) in batch.iter().zip(&results) {
        if matches!(
            pending.command,
            Command::OpenText { .. }
                | Command::OpenEvents { .. }
                | Command::OpenActivityOutput { .. }
        ) && let Ok(receipt) = result
            && receipt.position.sequence == sequence
            && let Mutation::Content(head) = &receipt.value
        {
            match content::stored_head(&mut transaction, head.id).await {
                Ok(final_head) => {
                    opened_heads.insert(head.id, final_head);
                }
                Err(error) => {
                    let error = abort_transaction(transaction).await.err().unwrap_or(error);
                    return aborted_batch(results, batch.len(), sequence, error);
                }
            }
        }
    }
    let fence = cache.pause();
    #[cfg(test)]
    for pending in batch {
        crash_window_tests::hit("command.before_commit", Some(pending.request));
    }
    let committed = transaction.commit().await;
    #[cfg(test)]
    let committed = crash_window_tests::lose_confirmation(committed, batch, cache);
    match committed {
        Ok(()) => {
            #[cfg(test)]
            for pending in batch {
                crash_window_tests::hit("command.after_commit", Some(pending.request));
                crash_window_tests::hit("command.before_install", Some(pending.request));
            }
            fence.committed(
                if shared_family { None } else { Some(&affected) },
                &workspaces,
            );
            metrics::add(&live.diagnostics().0.command_transactions, 1);
            for (ordinal, ((pending, result), updates)) in
                batch.iter().zip(&results).zip(changes).enumerate()
            {
                if let Ok(receipt) = result {
                    // Durable replays keep their original position and do not
                    // publish duplicate changes in a later transaction.
                    if receipt.position
                        == (CommitPosition {
                            sequence,
                            ordinal: ordinal as u32,
                        })
                    {
                        metrics::add(&live.diagnostics().0.command_results, 1);
                        if let Mutation::BlobPin(pin) = &receipt.value {
                            blob_pins.install(pin.lease);
                        }
                        if let (
                            Command::OpenText {
                                scope, round, tool, ..
                            },
                            Mutation::Content(head),
                        ) = (&pending.command, &receipt.value)
                        {
                            live.install_open(
                                port::OutputScope::Attempt(*scope),
                                Some(*round),
                                *tool,
                                opened_heads
                                    .get(&head.id)
                                    .expect("newly opened final head")
                                    .clone(),
                            );
                        }
                        if let (
                            Command::OpenEvents {
                                scope, round, tool, ..
                            },
                            Mutation::Content(head),
                        ) = (&pending.command, &receipt.value)
                        {
                            live.install_open(
                                port::OutputScope::Attempt(*scope),
                                Some(*round),
                                Some(*tool),
                                opened_heads
                                    .get(&head.id)
                                    .expect("newly opened final head")
                                    .clone(),
                            );
                        }
                        if let (
                            Command::OpenActivityOutput { scope, .. },
                            Mutation::Content(head),
                        ) = (&pending.command, &receipt.value)
                        {
                            live.install_open(
                                port::OutputScope::Activity(*scope),
                                None,
                                None,
                                opened_heads
                                    .get(&head.id)
                                    .expect("newly opened final head")
                                    .clone(),
                            );
                        }
                        let (updates, entry_changes) = updates;
                        live.acknowledge(&updates);
                        #[cfg(test)]
                        crash_window_tests::hit("command.after_install", Some(pending.request));
                        if !matches!(
                            pending.command,
                            Command::PrepareBlob { .. }
                                | Command::PinBlob { .. }
                                | Command::PinSnapshotBlob { .. }
                                | Command::WriteBlob { .. }
                                | Command::ReadyBlob { .. }
                        ) {
                            let (session, kind) = change_target(&pending.command, &receipt.value);
                            #[cfg(test)]
                            crash_window_tests::hit("command.before_notify", Some(pending.request));
                            feed.publish_scoped(
                                receipt.position,
                                session,
                                entry_changes.workspace_id,
                                entry_changes.include_in_lists,
                                kind,
                                entry_changes.entries,
                                entry_changes.requires_snapshot,
                                entry_changes.list_change,
                            );
                            #[cfg(test)]
                            crash_window_tests::hit("command.after_notify", Some(pending.request));
                        }
                    }
                }
            }
            if shared_family {
                feed.invalidate();
            }
            results
        }
        Err(error) => {
            if !rejected_commit(&error) {
                cache.close();
            }
            results
                .into_iter()
                .zip(batch)
                .map(|(result, pending)| match result {
                    Ok(receipt) if receipt.position.sequence != sequence => Ok(receipt),
                    Ok(_) => Err(match &error {
                        // A rejected constraint or busy COMMIT has not committed.
                        // Transport/I/O failures retain the uncertain category.
                        sqlx::Error::Database(database)
                            if database
                                .code()
                                .and_then(|code| code.parse::<u32>().ok())
                                .is_some_and(|code| matches!(code & 0xff, 5 | 6 | 19)) =>
                        {
                            storage(&error)
                        }
                        _ => KernelError::CommitUnknown {
                            request_id: pending.request,
                            diagnostic: error.to_string(),
                        },
                    }),
                    Err(error) => Err(error),
                })
                .collect()
        }
    }
}

fn aborted_batch(
    mut results: Vec<Reply>,
    length: usize,
    sequence: CommitSequence,
    error: KernelError,
) -> Vec<Reply> {
    results.resize_with(length, || Err(error.clone()));
    results
        .into_iter()
        .map(|result| match result {
            Ok(receipt) if receipt.position.sequence != sequence => Ok(receipt),
            _ => Err(error.clone()),
        })
        .collect()
}

fn previous_list_session(command: &Command) -> Option<SessionId> {
    Some(match command {
        Command::CreateConfigured { .. } | Command::CreateSession { .. } | Command::Fork { .. } => {
            return None;
        }
        Command::PresentInteraction { session, .. }
        | Command::Configure { session, .. }
        | Command::Withdraw { session, .. }
        | Command::DeleteSession { session }
        | Command::Continue { session, .. }
        | Command::Deliver { session, .. }
        | Command::DeliverToExecution { session, .. }
        | Command::ResumeDelivery { session, .. }
        | Command::ConsumeDelivery { session, .. }
        | Command::Metadata { session, .. }
        | Command::Selection { session, .. }
        | Command::Send { session, .. }
        | Command::Claim { session, .. }
        | Command::Cancel { session, .. }
        | Command::CancelQueued { session, .. }
        | Command::Retry { session, .. }
        | Command::Reply { session, .. }
        | Command::PrepareBlob { session, .. }
        | Command::PinBlob { session, .. }
        | Command::PruneContent { session, .. } => *session,
        Command::Notice { scope, .. }
        | Command::RecordCompaction { scope, .. }
        | Command::BeginRound { scope, .. }
        | Command::BeginCurrentRound { scope }
        | Command::BeginCompactionRound { scope, .. }
        | Command::FinishRound { scope, .. }
        | Command::CommitCompaction { scope, .. }
        | Command::BeginTool { scope, .. }
        | Command::FinalizeTool { scope, .. }
        | Command::RequestInteraction { scope, .. }
        | Command::OpenText { scope, .. }
        | Command::OpenEvents { scope, .. }
        | Command::BeginActivity { scope, .. }
        | Command::CompleteTool { scope, .. }
        | Command::PrepareStop { scope, .. }
        | Command::Finish { scope, .. } => scope.session,
        Command::EndActivity { scope, .. } | Command::OpenActivityOutput { scope, .. } => {
            scope.session
        }
        Command::SealContent { scope, .. } => scope.session(),
        Command::PinSnapshotBlob { snapshot, .. } => snapshot.session,
        Command::WriteBlob { lease, .. } | Command::ReadyBlob { lease, .. } => lease.session,
    })
}

pub(super) async fn list_state(
    connection: &mut SqliteConnection,
    session: SessionId,
) -> Result<Option<SessionListState>> {
    let statement = format!(
        "SELECT s.workspace_id,s.parent_session_id,s.favorite,s.pinned,s.temporary=0 AS include_in_lists,s.relation_kind='subagent' AS is_subagent,s.metadata_version,s.updated_at_ms,t.queued_executions,t.running_executions,t.user_message_count,t.last_message_at_ms,({}) AS processing_state FROM agena_sessions s JOIN agena_session_statistics t ON t.session_id=s.id WHERE s.id=? AND s.deleted_at_ms IS NULL",
        lifecycle::STATE_SQL
    );
    let Some(row) = sqlx::query(&statement)
        .bind(session.get())
        .fetch_optional(connection)
        .await
        .map_err(sql_error)?
    else {
        return Ok(None);
    };
    let count = |name| {
        row.try_get::<i64, _>(name)
            .map_err(storage)
            .and_then(|value| u64::try_from(value).map_err(storage))
    };
    Ok(Some(SessionListState {
        session,
        workspace_id: row.try_get("workspace_id").map_err(storage)?,
        parent_session_id: row
            .try_get::<Option<i64>, _>("parent_session_id")
            .map_err(storage)?
            .map(SessionId::try_from)
            .transpose()
            .map_err(storage)?,
        favorite: row.try_get("favorite").map_err(storage)?,
        pinned: row.try_get("pinned").map_err(storage)?,
        include_in_lists: row.try_get("include_in_lists").map_err(storage)?,
        is_subagent: row.try_get("is_subagent").map_err(storage)?,
        processing_state: agena_domain::SessionStateKind::parse(
            row.try_get("processing_state").map_err(storage)?,
        )
        .ok_or_else(|| storage("invalid session state"))?,
        queued_executions: count("queued_executions")?,
        running_executions: count("running_executions")?,
        metadata_version: row.try_get("metadata_version").map_err(storage)?,
        updated_at_ms: row.try_get("updated_at_ms").map_err(storage)?,
        user_message_count: count("user_message_count")?,
        last_message_at_ms: row.try_get("last_message_at_ms").map_err(storage)?,
    }))
}

fn change_target(command: &Command, mutation: &Mutation) -> (SessionId, ChangeKind) {
    match command {
        Command::RecordCompaction { scope, .. } => (scope.session, ChangeKind::Round),
        Command::CreateConfigured { .. } | Command::CreateSession { .. } => match mutation {
            Mutation::Session(metadata) => (metadata.id, ChangeKind::Metadata),
            _ => unreachable!("validated create-session result"),
        },
        Command::PresentInteraction { session, .. } => (*session, ChangeKind::Interaction),
        Command::Fork { session, .. } => match mutation {
            Mutation::Session(value) => (value.id, ChangeKind::Membership),
            Mutation::Rewind(value) => (value.session.id, ChangeKind::Membership),
            _ => (*session, ChangeKind::Membership),
        },
        Command::Configure { session, .. } => (*session, ChangeKind::Metadata),
        Command::Withdraw { session, .. }
        | Command::DeleteSession { session }
        | Command::ConsumeDelivery { session, .. } => (*session, ChangeKind::Membership),
        Command::Continue { session, .. } | Command::ResumeDelivery { session, .. } => {
            (*session, ChangeKind::Execution)
        }
        Command::Notice { scope, .. } => (scope.session, ChangeKind::Membership),
        Command::Deliver { session, .. } | Command::DeliverToExecution { session, .. } => {
            (*session, ChangeKind::Activity)
        }
        Command::Metadata { session, .. } | Command::Selection { session, .. } => {
            (*session, ChangeKind::Metadata)
        }
        Command::Send { session, .. } => (*session, ChangeKind::Membership),
        Command::Claim { session, .. }
        | Command::Cancel { session, .. }
        | Command::CancelQueued { session, .. }
        | Command::Retry { session, .. } => (*session, ChangeKind::Execution),
        Command::BeginRound { scope, .. }
        | Command::BeginCurrentRound { scope }
        | Command::BeginCompactionRound { scope, .. }
        | Command::FinishRound { scope, .. } => (scope.session, ChangeKind::Round),
        Command::CommitCompaction { scope, .. } => (scope.session, ChangeKind::Membership),
        Command::BeginTool { scope, .. }
        | Command::FinalizeTool { scope, .. }
        | Command::RequestInteraction { scope, .. } => (scope.session, ChangeKind::Membership),
        Command::OpenText { scope, tool, .. } => (
            scope.session,
            if tool.is_some() {
                ChangeKind::Content
            } else {
                ChangeKind::Membership
            },
        ),
        Command::OpenEvents { scope, .. } => (scope.session, ChangeKind::Content),
        Command::SealContent { scope, .. } => (scope.session(), ChangeKind::Content),
        Command::BeginActivity { scope, .. } => (scope.session, ChangeKind::Activity),
        Command::EndActivity { scope, .. } => (scope.session, ChangeKind::Activity),
        Command::OpenActivityOutput { scope, .. } => (scope.session, ChangeKind::Content),
        Command::CompleteTool { scope, .. } => (scope.session, ChangeKind::Tool),
        Command::PrepareBlob { session, .. } | Command::PinBlob { session, .. } => {
            (*session, ChangeKind::Content)
        }
        Command::PinSnapshotBlob { snapshot, .. } => (snapshot.session, ChangeKind::Content),
        Command::WriteBlob { lease, .. } | Command::ReadyBlob { lease, .. } => {
            (lease.session, ChangeKind::Content)
        }
        Command::PruneContent { session, .. } => (*session, ChangeKind::Content),
        Command::PrepareStop { scope, .. } | Command::Finish { scope, .. } => {
            (scope.session, ChangeKind::Execution)
        }
        Command::Reply { session, .. } => (*session, ChangeKind::Interaction),
    }
}

async fn execute(
    connection: &mut SqliteConnection,
    pending: &Pending,
    position: CommitPosition,
    live: &content::LiveContent,
) -> Result<(
    CommitReceipt<Mutation>,
    (Vec<content::StagedSource>, EntryChanges),
)> {
    let existing = sqlx::query(
        "SELECT request_digest,receipt_json FROM agena_command_results WHERE request_id=?",
    )
    .bind(pending.request.to_string())
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    if let Some(row) = existing {
        let digest: Vec<u8> = row.try_get("request_digest").map_err(storage)?;
        if digest != pending.digest {
            return Err(KernelError::Conflict(
                "request id was already used for another command".into(),
            ));
        }
        return Ok((
            decode(&row.try_get::<String, _>("receipt_json").map_err(storage)?)?,
            (Vec::new(), EntryChanges::default()),
        ));
    }
    // A changed compaction must validate its sealed summary while atomically
    // installing the checkpoint. Rollback preserves accepted live tails for a
    // retry, just as other failed terminal barriers do.
    let compaction = matches!(pending.command, Command::CommitCompaction { .. });
    let mut updates = if compaction {
        live.terminal(pending.request, &pending.command)
    } else {
        Vec::new()
    };
    if compaction {
        content::persist(connection, &mut updates).await?;
        content::seal_stored(connection, &pending.command).await?;
    }
    let before = match previous_list_session(&pending.command) {
        Some(session) => list_state(connection, session).await?,
        None => None,
    };
    let mut value = commands::apply(connection, &pending.command).await?;
    if !compaction {
        updates = live.terminal(pending.request, &pending.command);
        content::persist(connection, &mut updates).await?;
        content::seal_stored(connection, &pending.command).await?;
    }
    if let Command::SealContent { content, .. } = pending.command {
        value = Mutation::Content(content::stored_head(connection, content).await?);
    }
    let session = change_target(&pending.command, &value).0;
    if sqlx::query("UPDATE agena_sessions SET control_version=control_version+1 WHERE id=? AND control_version<9223372036854775807")
        .bind(session.get()).execute(&mut *connection).await.map_err(sql_error)?.rows_affected() != 1
    {
        return Err(KernelError::Conflict("session control version exhausted".into()));
    }
    let mut entry_changes = entry_changes(connection, &pending.command, &value).await?;
    let after = list_state(connection, session).await?;
    let mut parents = Vec::new();
    let old_parent = before.as_ref().and_then(|state| state.parent_session_id);
    let new_parent = after.as_ref().and_then(|state| state.parent_session_id);
    if before.is_none() != after.is_none()
        || old_parent != new_parent
        || before.as_ref().map(|state| state.include_in_lists)
            != after.as_ref().map(|state| state.include_in_lists)
    {
        for parent in [old_parent, new_parent].into_iter().flatten() {
            if parents
                .iter()
                .any(|state: &SessionListState| state.session == parent)
            {
                continue;
            }
            if let Some(state) = list_state(connection, parent).await? {
                parents.push(state);
            }
        }
    }
    entry_changes.list_change = Some(SessionListChange {
        before,
        after,
        parents,
    });
    let receipt = CommitReceipt { position, value };
    sqlx::query("INSERT INTO agena_command_results (request_id,request_digest,receipt_json,commit_sequence,command_ordinal,created_at_ms) VALUES (?,?,?,?,?,?)")
        .bind(pending.request.to_string()).bind(&pending.digest).bind(encode(&receipt)?)
        .bind(position.sequence.get()).bind(i64::from(position.ordinal)).bind(now_ms())
        .execute(&mut *connection).await.map_err(sql_error)?;
    Ok((receipt, (updates, entry_changes)))
}

#[derive(Default)]
struct EntryChanges {
    entries: Vec<EntryId>,
    requires_snapshot: bool,
    workspace_id: i64,
    include_in_lists: bool,
    list_change: Option<SessionListChange>,
}
async fn entry_changes(
    connection: &mut SqliteConnection,
    command: &Command,
    value: &Mutation,
) -> Result<EntryChanges> {
    let session = change_target(command, value).0;
    let context = sqlx::query(
        "SELECT workspace_id,temporary=0 AS include_in_lists FROM agena_sessions WHERE id=?",
    )
    .bind(session.get())
    .fetch_one(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut change = EntryChanges {
        workspace_id: context.try_get("workspace_id").map_err(storage)?,
        include_in_lists: context.try_get("include_in_lists").map_err(storage)?,
        ..Default::default()
    };
    match command {
        Command::Fork { .. } | Command::Withdraw { .. } | Command::DeleteSession { .. } => {
            change.requires_snapshot = true;
            return Ok(change);
        }
        _ => {}
    }
    let ids:Vec<i64>=match value {
        Mutation::Execution(view)=>sqlx::query_scalar("SELECT id FROM agena_entries WHERE id=? OR (execution_id=? AND generation=?) ORDER BY id LIMIT 129").bind(view.message_id.get()).bind(view.id.to_string()).bind(view.generation.get()).fetch_all(&mut *connection).await.map_err(sql_error)?,
        Mutation::Tool(id)=>sqlx::query_scalar("SELECT id FROM agena_entries WHERE tool_id=? ORDER BY id LIMIT 129").bind(id.get()).fetch_all(&mut *connection).await.map_err(sql_error)?,
        Mutation::Interaction(id)=>sqlx::query_scalar("SELECT id FROM agena_entries WHERE interaction_id=? OR tool_id=(SELECT tool_id FROM agena_interactions WHERE id=?) ORDER BY id LIMIT 129").bind(id.get()).bind(id.get()).fetch_all(&mut *connection).await.map_err(sql_error)?,
        Mutation::InteractionView(view)=>sqlx::query_scalar("SELECT id FROM agena_entries WHERE interaction_id=? OR tool_id=? ORDER BY id LIMIT 129").bind(view.id.get()).bind(view.tool_id.get()).fetch_all(&mut *connection).await.map_err(sql_error)?,
        Mutation::Decision(_)=>if let Command::Reply{interaction,..}=command {sqlx::query_scalar("SELECT id FROM agena_entries WHERE interaction_id=? OR tool_id=(SELECT tool_id FROM agena_interactions WHERE id=?) ORDER BY id LIMIT 129").bind(interaction.get()).bind(interaction.get()).fetch_all(&mut *connection).await.map_err(sql_error)?}else{Vec::new()},
        Mutation::Content(head)=>vec![head.entry.get()],
        Mutation::Retention(retention)=>vec![retention.stream.entry.get()],
        Mutation::Checkpoint(id)=>vec![id.get()],
        Mutation::Activity(id)=>sqlx::query_scalar("SELECT entry_id FROM agena_background_activities WHERE id=?").bind(id.to_string()).fetch_all(&mut *connection).await.map_err(sql_error)?,
        _=>Vec::new(),
    };
    if ids.len() > 128 {
        change.requires_snapshot = true;
    } else {
        change.entries = ids
            .into_iter()
            .map(EntryId::try_from)
            .collect::<std::result::Result<_, _>>()
            .map_err(storage)?;
    }
    Ok(change)
}
