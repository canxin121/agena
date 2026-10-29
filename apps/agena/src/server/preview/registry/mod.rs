use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use url::{Host, Url};

use crate::{ApiResult, AppError};

use crate::server::persistence::db;

const CACHE_TTL: Duration = Duration::from_millis(750);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreviewSessionRecord {
    pub(crate) id: String,
    /// Server-generated identity of this registration. Reusing a display ID
    /// after deletion must never revive an old capability for that ID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) instance_id: String,
    pub(crate) directory: String,
    pub(crate) run_directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agena_session_id: Option<String>,
    pub(crate) state: String,
    pub(crate) proxy_base_path: String,
    pub(crate) target_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) logs_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<i64>,
    pub(crate) updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) framework_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreviewSessionCreateRequest {
    pub(crate) id: String,
    pub(crate) directory: String,
    pub(crate) run_directory: String,
    pub(crate) agena_session_id: Option<String>,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) logs_path: String,
    pub(crate) target_url: Url,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PreviewSessionUpdatePatch {
    pub(crate) directory: Option<String>,
    pub(crate) run_directory: Option<String>,
    pub(crate) agena_session_id: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) args: Option<Vec<String>>,
    pub(crate) logs_path: Option<String>,
    pub(crate) target_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewSessionsResponse {
    pub(crate) updated_at: i64,
    pub(crate) sessions: Vec<PreviewSessionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewSessionsFile {
    updated_at: i64,
    sessions: Vec<PreviewSessionRecord>,
}

#[derive(Debug, Clone)]
struct RegistryCache {
    revision: u64,
    loaded_at: Instant,
    server_state_db_path: PathBuf,
    snapshot: PreviewSessionsResponse,
}

/// Build and serialize once per registry revision, shared by every browser.
/// Slow consumers receive a reset when they missed the delta's base revision.
#[derive(Debug)]
struct PreviewEventSnapshot {
    revision: String,
    base_revision: Option<String>,
    snapshot: PreviewSessionsResponse,
    delta: Arc<str>,
    reset: Option<Arc<str>>,
}

impl PreviewEventSnapshot {
    fn new(revision: String, previous: Option<&Self>, snapshot: PreviewSessionsResponse) -> Self {
        let delta: Arc<str> =
            serde_json::json!({"type":"preview.sessions.changed", "revision":revision,
            "delta":preview_session_delta(previous.map(|old| &old.snapshot), &snapshot)})
            .to_string()
            .into();
        Self {
            revision,
            base_revision: previous.map(|old| old.revision.clone()),
            snapshot,
            reset: previous.is_none().then(|| delta.clone()),
            delta,
        }
    }

    fn payload(&mut self, previous: Option<&str>) -> Option<Arc<str>> {
        if previous == Some(self.revision.as_str()) {
            return None;
        }
        if previous.is_some() && previous == self.base_revision.as_deref() {
            return Some(self.delta.clone());
        }
        let reset = self.reset.get_or_insert_with(|| {
            serde_json::json!({"type":"preview.sessions.changed",
            "revision":self.revision, "delta":preview_session_delta(None, &self.snapshot)})
            .to_string()
            .into()
        });
        Some(reset.clone())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspacePreviewRegistry {
    db: Arc<crate::server::persistence::db::ServerStateDb>,
    ttl: Duration,
    cache: Arc<RwLock<Option<RegistryCache>>>,
    epoch: uuid::Uuid,
    changes: tokio::sync::watch::Sender<u64>,
    events: Arc<tokio::sync::Mutex<Option<PreviewEventSnapshot>>>,
    mutations: Arc<tokio::sync::Mutex<()>>,
}

impl WorkspacePreviewRegistry {
    pub(crate) fn new(db: Arc<crate::server::persistence::db::ServerStateDb>) -> Self {
        Self::new_with_ttl(db, CACHE_TTL)
    }

    fn new_with_ttl(db: Arc<crate::server::persistence::db::ServerStateDb>, ttl: Duration) -> Self {
        Self {
            db,
            ttl,
            cache: Arc::new(RwLock::new(None)),
            epoch: uuid::Uuid::new_v4(),
            changes: tokio::sync::watch::channel(0).0,
            events: Arc::new(tokio::sync::Mutex::new(None)),
            mutations: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) async fn list_all(&self) -> ApiResult<PreviewSessionsResponse> {
        self.try_snapshot().await
    }

    pub(crate) async fn get_by_id(&self, id: &str) -> Option<PreviewSessionRecord> {
        self.snapshot()
            .await
            .sessions
            .into_iter()
            .find(|session| session.id == id)
    }

    pub(crate) async fn invalidate(&self) {
        let mut cache = self.cache.write().await;
        *cache = None;
        self.changes.send_modify(|revision| *revision += 1);
    }

    pub(crate) fn revision(&self) -> String {
        format!("{}:{}", self.epoch, *self.changes.borrow())
    }

    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(crate) async fn event_payload(
        &self,
        previous: Option<&str>,
    ) -> ApiResult<(String, Option<Arc<str>>)> {
        let mut events = self.events.lock().await;
        let revision = self.revision();
        if events
            .as_ref()
            .is_none_or(|cached| cached.revision != revision)
        {
            // Never memoize a failed database read as an authoritative empty
            // registry for the rest of this revision's lifetime.
            let snapshot = self.try_snapshot().await?;
            *events = Some(PreviewEventSnapshot::new(
                revision.clone(),
                events.as_ref(),
                snapshot,
            ));
        }
        Ok((
            revision,
            events
                .as_mut()
                .expect("preview event snapshot")
                .payload(previous),
        ))
    }

    // Once a write is admitted, its read/modify/write lock and invalidation
    // belong to the operation, not to the HTTP caller. SQLite can commit after
    // a waiting request is cancelled; dropping this work would lose revisions
    // or let the next mutation overwrite that unobserved commit.
    pub(crate) async fn create_server_session(
        &self,
        request: PreviewSessionCreateRequest,
    ) -> ApiResult<PreviewSessionRecord> {
        let registry = self.clone();
        finish_mutation(async move { registry.create_server_session_inner(request).await }).await
    }

    pub(crate) async fn delete_by_id(&self, id: &str) -> ApiResult<()> {
        let registry = self.clone();
        let id = id.to_owned();
        finish_mutation(async move { registry.delete_by_id_inner(&id).await }).await
    }

    pub(crate) async fn update_by_id(
        &self,
        id: &str,
        patch: PreviewSessionUpdatePatch,
    ) -> ApiResult<PreviewSessionRecord> {
        let registry = self.clone();
        let id = id.to_owned();
        finish_mutation(async move { registry.update_by_id_inner(&id, patch).await }).await
    }

    pub(crate) async fn rename_by_id(
        &self,
        id: &str,
        new_id: &str,
    ) -> ApiResult<PreviewSessionRecord> {
        let registry = self.clone();
        let id = id.to_owned();
        let new_id = new_id.to_owned();
        finish_mutation(async move { registry.rename_by_id_inner(&id, &new_id).await }).await
    }

    pub(crate) async fn mark_running_by_id(
        &self,
        id: &str,
        pid: u32,
    ) -> ApiResult<PreviewSessionRecord> {
        let registry = self.clone();
        let id = id.to_owned();
        finish_mutation(async move { registry.mark_running_by_id_inner(&id, pid).await }).await
    }

    pub(crate) async fn mark_stopped_by_id(
        &self,
        id: &str,
        error: Option<String>,
    ) -> ApiResult<PreviewSessionRecord> {
        let registry = self.clone();
        let id = id.to_owned();
        finish_mutation(async move { registry.mark_stopped_by_id_inner(&id, error, None).await })
            .await
    }

    /// Preserve evidence of an owned launch even when its running-state write
    /// failed, or an older imported running record omitted its start time.
    pub(crate) async fn mark_owned_stopped_by_id(
        &self,
        id: &str,
        error: Option<String>,
        started_at: i64,
    ) -> ApiResult<PreviewSessionRecord> {
        let registry = self.clone();
        let id = id.to_owned();
        finish_mutation(async move {
            registry
                .mark_stopped_by_id_inner(&id, error, Some(started_at))
                .await
        })
        .await
    }

    async fn load_server_state_file(&self) -> ApiResult<PreviewSessionsFile> {
        match self
            .db
            .get_json::<PreviewSessionsFile>(
                crate::server::persistence::db::KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE,
            )
            .await
        {
            Ok(Some(file)) => Ok(file),
            Ok(None) => Ok(PreviewSessionsFile {
                updated_at: 0,
                sessions: Vec::new(),
            }),
            Err(err) => Err(AppError::internal(format!(
                "failed to read server preview registry from db: {err}"
            ))),
        }
    }

    async fn write_server_state_file(&self, file: &PreviewSessionsFile) -> ApiResult<()> {
        self.db
            .set_json(
                crate::server::persistence::db::KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE,
                file.clone(),
            )
            .await
            .map_err(|err| {
                AppError::internal(format!("failed to persist server preview registry: {err}"))
            })?;
        Ok(())
    }

    async fn remove_session_from_server_store(&self, id: &str, updated_at: i64) -> ApiResult<bool> {
        let mut file = self.load_server_state_file().await?;
        let removed = remove_session_from_file(&mut file, id, updated_at);
        if !removed {
            return Ok(false);
        }
        self.write_server_state_file(&file).await?;
        Ok(true)
    }

    async fn update_session_in_server_store(
        &self,
        id: &str,
        patch: PreviewSessionUpdatePatch,
        updated_at: i64,
    ) -> ApiResult<Option<PreviewSessionRecord>> {
        let mut file = self.load_server_state_file().await?;
        let updated = update_session_in_file(&mut file, id, patch, updated_at)?;
        if updated.is_some() {
            self.write_server_state_file(&file).await?;
        }
        Ok(updated)
    }

    async fn rename_session_in_server_store(
        &self,
        id: &str,
        new_id: &str,
        updated_at: i64,
    ) -> ApiResult<Option<PreviewSessionRecord>> {
        let mut file = self.load_server_state_file().await?;
        let updated = rename_session_in_file(&mut file, id, new_id, updated_at)?;
        if updated.is_some() {
            self.write_server_state_file(&file).await?;
        }
        Ok(updated)
    }

    async fn mark_running_in_server_store(
        &self,
        id: &str,
        pid: u32,
        updated_at: i64,
    ) -> ApiResult<Option<PreviewSessionRecord>> {
        let mut file = self.load_server_state_file().await?;
        let updated = mark_running_in_file(&mut file, id, pid, updated_at)?;
        if updated.is_some() {
            self.write_server_state_file(&file).await?;
        }
        Ok(updated)
    }

    async fn mark_stopped_in_server_store(
        &self,
        id: &str,
        error: Option<String>,
        updated_at: i64,
        started_at: Option<i64>,
    ) -> ApiResult<Option<PreviewSessionRecord>> {
        let mut file = self.load_server_state_file().await?;
        let updated = mark_stopped_in_file(&mut file, id, error, updated_at, started_at)?;
        if updated.is_some() {
            self.write_server_state_file(&file).await?;
        }
        Ok(updated)
    }

    async fn create_server_session_inner(
        &self,
        request: PreviewSessionCreateRequest,
    ) -> ApiResult<PreviewSessionRecord> {
        let _mutation = self.mutations.lock().await;
        let PreviewSessionCreateRequest {
            id,
            directory,
            run_directory,
            agena_session_id,
            command,
            args,
            logs_path,
            target_url,
        } = request;
        let trimmed_id = id.trim();
        if trimmed_id.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }
        if !is_valid_preview_session_id(trimmed_id) {
            return Err(AppError::bad_request(
                "invalid preview session id (use ASCII letters, numbers, '_' or '-')",
            ));
        }

        let mut file = self.load_server_state_file().await?;
        if file.sessions.iter().any(|session| session.id == trimmed_id) {
            return Err(AppError::bad_request(format!(
                "preview session already exists: {trimmed_id}"
            )));
        }

        let updated_at = now_millis();

        let trimmed_directory = directory.trim();
        if trimmed_directory.is_empty() {
            return Err(AppError::bad_request("directory is required"));
        }
        let trimmed_run_directory = run_directory.trim();
        let resolved_run_directory = if trimmed_run_directory.is_empty() {
            trimmed_directory
        } else {
            trimmed_run_directory
        };
        let trimmed_command = command.trim();
        if trimmed_command.is_empty() {
            return Err(AppError::bad_request("command is required"));
        }
        let trimmed_logs_path = logs_path.trim();
        let resolved_logs_path = if trimmed_logs_path.is_empty() {
            default_preview_logs_path(self.db.path(), trimmed_id)
        } else {
            trimmed_logs_path.to_string()
        };

        let agena_session_id = agena_session_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let record = PreviewSessionRecord {
            id: trimmed_id.to_string(),
            instance_id: uuid::Uuid::new_v4().to_string(),
            directory: trimmed_directory.to_string(),
            run_directory: resolved_run_directory.to_string(),
            agena_session_id,
            state: "stopped".to_string(),
            proxy_base_path: preview_proxy_base_path(trimmed_id),
            target_url: target_url.to_string(),
            pid: None,
            port: target_url.port_or_known_default(),
            command: trimmed_command.to_string(),
            args,
            logs_path: resolved_logs_path,
            started_at: None,
            updated_at,
            framework_hint: None,
            error: None,
        };

        file.updated_at = updated_at;
        file.sessions.push(record.clone());
        self.write_server_state_file(&file).await?;
        self.invalidate().await;
        Ok(record)
    }

    async fn delete_by_id_inner(&self, id: &str) -> ApiResult<()> {
        let _mutation = self.mutations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let updated_at = now_millis();

        if !self
            .remove_session_from_server_store(trimmed, updated_at)
            .await?
        {
            return Err(AppError::not_found(format!(
                "preview session not found: {trimmed}"
            )));
        }

        self.invalidate().await;
        Ok(())
    }

    async fn update_by_id_inner(
        &self,
        id: &str,
        patch: PreviewSessionUpdatePatch,
    ) -> ApiResult<PreviewSessionRecord> {
        let _mutation = self.mutations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let updated_at = now_millis();

        if let Some(updated) = self
            .update_session_in_server_store(trimmed, patch.clone(), updated_at)
            .await?
        {
            self.invalidate().await;
            return Ok(updated);
        }

        Err(AppError::not_found(format!(
            "preview session not found: {trimmed}"
        )))
    }

    async fn rename_by_id_inner(&self, id: &str, new_id: &str) -> ApiResult<PreviewSessionRecord> {
        let _mutation = self.mutations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let trimmed_new = new_id.trim();
        if trimmed_new.is_empty() {
            return Err(AppError::bad_request("new id is required"));
        }
        if !is_valid_preview_session_id(trimmed_new) {
            return Err(AppError::bad_request(
                "invalid preview session id (use ASCII letters, numbers, '_' or '-')",
            ));
        }

        if trimmed == trimmed_new {
            return self.get_by_id(trimmed).await.ok_or_else(|| {
                AppError::not_found(format!("preview session not found: {trimmed}"))
            });
        }

        let server_file = self.load_server_state_file().await?;
        if server_file
            .sessions
            .iter()
            .any(|session| session.id == trimmed_new)
        {
            return Err(AppError::bad_request(format!(
                "preview session already exists: {trimmed_new}"
            )));
        }

        let updated_at = now_millis();

        let updated = self
            .rename_session_in_server_store(trimmed, trimmed_new, updated_at)
            .await?
            .ok_or_else(|| AppError::not_found(format!("preview session not found: {trimmed}")))?;

        self.invalidate().await;
        Ok(updated)
    }

    async fn mark_running_by_id_inner(
        &self,
        id: &str,
        pid: u32,
    ) -> ApiResult<PreviewSessionRecord> {
        let _mutation = self.mutations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let updated_at = now_millis();

        if let Some(updated) = self
            .mark_running_in_server_store(trimmed, pid, updated_at)
            .await?
        {
            self.invalidate().await;
            return Ok(updated);
        }

        Err(AppError::not_found(format!(
            "preview session not found: {trimmed}"
        )))
    }

    async fn mark_stopped_by_id_inner(
        &self,
        id: &str,
        error: Option<String>,
        started_at: Option<i64>,
    ) -> ApiResult<PreviewSessionRecord> {
        let _mutation = self.mutations.lock().await;
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let updated_at = now_millis();

        if let Some(updated) = self
            .mark_stopped_in_server_store(trimmed, error.clone(), updated_at, started_at)
            .await?
        {
            self.invalidate().await;
            return Ok(updated);
        }

        Err(AppError::not_found(format!(
            "preview session not found: {trimmed}"
        )))
    }

    async fn snapshot(&self) -> PreviewSessionsResponse {
        self.try_snapshot().await.unwrap_or_else(|error| {
            tracing::warn!(target: "agena.preview_registry", error = %error,
                "Failed to load server preview registry from db");
            empty_snapshot()
        })
    }

    async fn try_snapshot(&self) -> ApiResult<PreviewSessionsResponse> {
        let revision = *self.changes.borrow();
        let server_state_db_path = self.db.path().to_path_buf();
        {
            let cache = self.cache.read().await;
            if let Some(cache) = cache.as_ref()
                && cache.revision == revision
                && cache.server_state_db_path == server_state_db_path
                && cache.loaded_at.elapsed() < self.ttl
            {
                return Ok(cache.snapshot.clone());
            }
        }

        let snapshot = parse_preview_sessions(self.load_server_state_file().await?);
        let cache_entry = RegistryCache {
            revision,
            loaded_at: Instant::now(),
            server_state_db_path,
            snapshot: snapshot.clone(),
        };

        let mut cache = self.cache.write().await;
        // A read started before a mutation cannot repopulate the cache that
        // mutation just invalidated, or tag the stale snapshot as current.
        if *self.changes.borrow() == revision {
            *cache = Some(cache_entry);
        }
        Ok(snapshot)
    }
}

async fn finish_mutation<T: Send + 'static>(
    operation: impl std::future::Future<Output = ApiResult<T>> + Send + 'static,
) -> ApiResult<T> {
    tokio::spawn(async move {
        let result = operation.await;
        if let Err(error) = &result {
            tracing::warn!(%error, "preview registry mutation failed");
        }
        result
    })
    .await
    .map_err(|error| {
        AppError::internal_error_with_context("complete preview registry mutation", &error)
    })?
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn remove_session_from_file(file: &mut PreviewSessionsFile, id: &str, updated_at: i64) -> bool {
    let before = file.sessions.len();
    file.sessions.retain(|session| session.id != id);
    if file.sessions.len() == before {
        return false;
    }
    file.updated_at = updated_at;
    true
}

fn update_session_in_file(
    file: &mut PreviewSessionsFile,
    id: &str,
    patch: PreviewSessionUpdatePatch,
    updated_at: i64,
) -> ApiResult<Option<PreviewSessionRecord>> {
    let Some(index) = file.sessions.iter().position(|session| session.id == id) else {
        return Ok(None);
    };

    let mut record = file.sessions[index].clone();

    if let Some(dir) = patch.directory {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("directory is required"));
        }
        record.directory = trimmed.to_string();
    }

    if let Some(dir) = patch.run_directory {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("runDirectory is required"));
        }
        record.run_directory = trimmed.to_string();
    }

    if let Some(value) = patch.agena_session_id {
        // Allow clearing by sending empty string.
        record.agena_session_id = normalize_optional_string(Some(value));
    }

    if let Some(cmd) = patch.command {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("command is required"));
        }
        record.command = trimmed.to_string();
    }

    if let Some(next) = patch.args {
        record.args = next;
    }

    if let Some(path) = patch.logs_path {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("logsPath is required"));
        }
        record.logs_path = trimmed.to_string();
    }

    if let Some(raw_target) = patch.target_url {
        let target = validate_preview_target_url(&raw_target)?;
        record.target_url = target.to_string();
        record.port = target.port_or_known_default();
    }

    record.updated_at = updated_at;
    record.proxy_base_path = preview_proxy_base_path(&record.id);

    file.sessions[index] = record.clone();
    file.updated_at = updated_at;
    Ok(Some(record))
}

fn mark_running_in_file(
    file: &mut PreviewSessionsFile,
    id: &str,
    pid: u32,
    updated_at: i64,
) -> ApiResult<Option<PreviewSessionRecord>> {
    let Some(index) = file.sessions.iter().position(|session| session.id == id) else {
        return Ok(None);
    };

    let mut record = file.sessions[index].clone();
    record.state = "running".to_string();
    record.pid = Some(pid);
    record.started_at = Some(updated_at);
    record.updated_at = updated_at;
    record.error = None;

    file.sessions[index] = record.clone();
    file.updated_at = updated_at;
    Ok(Some(record))
}

fn mark_stopped_in_file(
    file: &mut PreviewSessionsFile,
    id: &str,
    error: Option<String>,
    updated_at: i64,
    started_at: Option<i64>,
) -> ApiResult<Option<PreviewSessionRecord>> {
    let Some(index) = file.sessions.iter().position(|session| session.id == id) else {
        return Ok(None);
    };

    let mut record = file.sessions[index].clone();
    record.state = "stopped".to_string();
    record.pid = None;
    record.started_at = record.started_at.or(started_at);
    record.updated_at = updated_at;
    record.error = error.and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    file.sessions[index] = record.clone();
    file.updated_at = updated_at;
    Ok(Some(record))
}

fn rename_session_in_file(
    file: &mut PreviewSessionsFile,
    id: &str,
    new_id: &str,
    updated_at: i64,
) -> ApiResult<Option<PreviewSessionRecord>> {
    let mut updated: Option<PreviewSessionRecord> = None;
    let mut touched = false;
    for session in file.sessions.iter_mut() {
        if session.id != id {
            continue;
        }
        session.id = new_id.to_string();
        session.proxy_base_path = preview_proxy_base_path(&session.id);
        session.updated_at = updated_at;
        touched = true;
        if updated.is_none() {
            updated = Some(session.clone());
        }
    }

    if !touched {
        return Ok(None);
    }

    file.updated_at = updated_at;
    Ok(updated)
}

fn empty_snapshot() -> PreviewSessionsResponse {
    PreviewSessionsResponse {
        updated_at: 0,
        sessions: Vec::new(),
    }
}

fn parse_preview_sessions(file: PreviewSessionsFile) -> PreviewSessionsResponse {
    let mut sessions = Vec::with_capacity(file.sessions.len());
    for session in file.sessions {
        if !is_valid_preview_session_id(&session.id) {
            continue;
        }
        if !is_valid_proxy_base_path(&session.id, &session.proxy_base_path) {
            continue;
        }
        if session.directory.trim().is_empty() {
            continue;
        }
        if session.run_directory.trim().is_empty() {
            continue;
        }
        if session.command.trim().is_empty() {
            continue;
        }
        if session.logs_path.trim().is_empty() {
            continue;
        }
        if session.target_url.trim().is_empty() {
            continue;
        }
        if validate_preview_target_url(&session.target_url).is_err() {
            continue;
        }
        sessions.push(session);
    }
    PreviewSessionsResponse {
        updated_at: file.updated_at,
        sessions,
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn default_preview_logs_path(server_state_db_path: &Path, id: &str) -> String {
    let root = server_state_db_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    root.join("workspace-preview")
        .join("logs")
        .join(format!("{id}.log"))
        .to_string_lossy()
        .into_owned()
}

fn is_valid_preview_session_id(id: &str) -> bool {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Keep IDs URL-safe: used in /api/v1/workbench/preview/s/{id}/ proxy paths.
    trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn is_valid_proxy_base_path(id: &str, proxy_base_path: &str) -> bool {
    proxy_base_path == preview_proxy_base_path(id)
}

pub(crate) fn validate_preview_target_url(raw: &str) -> ApiResult<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("target is required"));
    }

    let url = Url::parse(trimmed)
        .map_err(|err| AppError::bad_request(format!("invalid target URL: {err}")))?;
    validate_preview_target(&url)
}

pub(crate) fn validate_preview_target(url: &Url) -> ApiResult<Url> {
    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(AppError::bad_request(
                "target URL must use http or https protocol",
            ));
        }
    }

    let host = url
        .host()
        .ok_or_else(|| AppError::bad_request("target URL host is required"))?;
    if !is_allowed_loopback_host(host) {
        return Err(AppError::bad_request(
            "target URL must use localhost, 127.0.0.1, or [::1]",
        ));
    }

    let mut normalized = url.clone();
    if normalized.path().is_empty() {
        normalized.set_path("/");
    }
    normalized.set_fragment(None);
    Ok(normalized)
}

fn is_allowed_loopback_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ipv4) => ipv4.octets() == [127, 0, 0, 1],
        Host::Ipv6(ipv6) => ipv6.is_loopback(),
    }
}

pub(crate) fn preview_proxy_base_path(id: &str) -> String {
    format!("/api/v1/workbench/preview/s/{id}/")
}

pub(crate) fn preview_target_from_record(record: &PreviewSessionRecord) -> ApiResult<Url> {
    let trimmed = record.target_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_gateway(
            "preview session is missing targetUrl",
        ));
    }
    validate_preview_target_url(trimmed)
}

pub(crate) fn build_proxy_target_url(
    target: &Url,
    request_path: Option<&str>,
    query: Option<&str>,
) -> Url {
    let mut upstream = target.clone();
    if let Some(path) = request_path {
        let trimmed = path.trim_start_matches('/');
        if !trimmed.is_empty() {
            let base = upstream.path().trim_end_matches('/');
            let next_path = if base.is_empty() || base == "/" {
                format!("/{trimmed}")
            } else {
                format!("{base}/{trimmed}")
            };
            upstream.set_path(&next_path);
        }
    }
    upstream.set_query(query);
    upstream
}

pub(crate) fn websocket_target_url(
    target: &Url,
    request_path: Option<&str>,
    query: Option<&str>,
) -> Url {
    let mut upstream = build_proxy_target_url(target, request_path, query);
    let scheme = match upstream.scheme() {
        "https" => "wss",
        _ => "ws",
    };
    let _ = upstream.set_scheme(scheme);
    upstream
}

fn preview_session_delta(
    previous: Option<&PreviewSessionsResponse>,
    snapshot: &PreviewSessionsResponse,
) -> serde_json::Value {
    let old = previous
        .into_iter()
        .flat_map(|snapshot| &snapshot.sessions)
        .map(|session| (session.id.as_str(), session))
        .collect::<std::collections::HashMap<_, _>>();
    let ids = snapshot
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let upsert = snapshot
        .sessions
        .iter()
        .filter(|session| old.get(session.id.as_str()).copied() != Some(*session))
        .collect::<Vec<_>>();
    let removed = old
        .keys()
        .filter(|id| !ids.contains(**id))
        .collect::<Vec<_>>();
    serde_json::json!({"reset": previous.is_none(), "upsert": upsert, "removed": removed})
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU8, Ordering};

    struct PausedStore {
        path: PathBuf,
        values: std::sync::RwLock<HashMap<String, String>>,
        pause: AtomicU8,
        blocked: tokio::sync::Notify,
        resume: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl agena_storage::instance_state::InstanceStateRepository for PausedStore {
        fn path(&self) -> &Path {
            &self.path
        }

        async fn get_payload(
            &self,
            key: &str,
        ) -> Result<Option<String>, agena_storage::instance_state::InstanceStateError> {
            Ok(self.values.read().unwrap().get(key).cloned())
        }

        async fn set_payload(
            &self,
            key: &str,
            payload: String,
        ) -> Result<(), agena_storage::instance_state::InstanceStateError> {
            let pause = self.pause.swap(0, Ordering::AcqRel);
            if pause == 1 {
                self.blocked.notify_one();
                self.resume.notified().await;
            }
            self.values.write().unwrap().insert(key.into(), payload);
            if pause == 2 {
                self.blocked.notify_one();
                self.resume.notified().await;
            }
            Ok(())
        }
    }

    async fn registry(root: &Path) -> (WorkspacePreviewRegistry, Arc<PausedStore>) {
        let store = Arc::new(PausedStore {
            path: root.join("registry-cancellation.sqlite3"),
            values: Default::default(),
            pause: AtomicU8::new(0),
            blocked: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        let db = Arc::new(db::ServerStateDb::from_store(store.clone()).await.unwrap());
        (
            WorkspacePreviewRegistry::new_with_ttl(db, Duration::from_secs(60)),
            store,
        )
    }

    fn request(root: &Path, id: &str) -> PreviewSessionCreateRequest {
        PreviewSessionCreateRequest {
            id: id.into(),
            directory: root.to_string_lossy().into_owned(),
            run_directory: root.to_string_lossy().into_owned(),
            agena_session_id: None,
            command: "node".into(),
            args: vec![],
            logs_path: root
                .join(format!("{id}.log"))
                .to_string_lossy()
                .into_owned(),
            target_url: Url::parse("http://127.0.0.1:39999/").unwrap(),
        }
    }

    #[tokio::test]
    async fn cancelled_caller_cannot_lose_a_committed_mutations_revision() {
        let root = tempfile::tempdir().unwrap();
        let (registry, store) = registry(root.path()).await;
        registry
            .create_server_session(request(root.path(), "preview"))
            .await
            .unwrap();
        let (before, _) = registry.event_payload(None).await.unwrap();
        let mut changes = registry.subscribe();
        store.pause.store(2, Ordering::Release);
        let updating = {
            let registry = registry.clone();
            tokio::spawn(async move {
                registry
                    .update_by_id(
                        "preview",
                        PreviewSessionUpdatePatch {
                            command: Some("node-next".into()),
                            ..Default::default()
                        },
                    )
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        updating.abort();
        assert!(updating.await.unwrap_err().is_cancelled());
        store.resume.notify_one();
        tokio::time::timeout(Duration::from_secs(2), changes.changed())
            .await
            .expect("a committed mutation must invalidate even after its caller disappears")
            .unwrap();
        let (after, payload) = registry.event_payload(None).await.unwrap();
        assert_ne!(before, after);
        assert!(payload.unwrap().contains("node-next"));
        assert_eq!(
            registry.get_by_id("preview").await.unwrap().command,
            "node-next"
        );
    }

    #[tokio::test]
    async fn cancelled_mutation_keeps_serialization_until_its_write_finishes() {
        let root = tempfile::tempdir().unwrap();
        let (registry, store) = registry(root.path()).await;
        store.pause.store(1, Ordering::Release);
        let first = {
            let registry = registry.clone();
            let request = request(root.path(), "first");
            tokio::spawn(async move { registry.create_server_session(request).await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        let mut second = {
            let registry = registry.clone();
            let request = request(root.path(), "second");
            tokio::spawn(async move { registry.create_server_session(request).await })
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(40), &mut second)
                .await
                .is_err(),
            "the next read-modify-write must not overtake an admitted write"
        );
        store.resume.notify_one();
        tokio::time::timeout(Duration::from_secs(2), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut ids = registry
            .list_all()
            .await
            .unwrap()
            .sessions
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["first", "second"]);
    }

    #[tokio::test]
    async fn owned_cleanup_preserves_launch_history_without_reclassifying_manual_attachments() {
        let root = tempfile::tempdir().unwrap();
        let (registry, _) = registry(root.path()).await;
        registry
            .create_server_session(request(root.path(), "owned"))
            .await
            .unwrap();
        registry
            .create_server_session(request(root.path(), "attached"))
            .await
            .unwrap();
        let owned = registry
            .mark_owned_stopped_by_id("owned", None, 123)
            .await
            .unwrap();
        assert_eq!(owned.started_at, Some(123));
        assert_eq!(owned.pid, None);
        assert_eq!(
            registry
                .mark_owned_stopped_by_id("owned", None, 456)
                .await
                .unwrap()
                .started_at,
            Some(123)
        );
        assert_eq!(
            registry
                .mark_stopped_by_id("attached", None)
                .await
                .unwrap()
                .started_at,
            None
        );
    }

    #[tokio::test]
    async fn recreating_the_same_preview_id_cannot_revive_an_old_run_identity() {
        let root = tempfile::tempdir().unwrap();
        let (registry, _) = registry(root.path()).await;
        let first = registry
            .create_server_session(request(root.path(), "reused"))
            .await
            .unwrap();
        registry.delete_by_id("reused").await.unwrap();
        let second = registry
            .create_server_session(request(root.path(), "reused"))
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.started_at, second.started_at);
        assert_ne!(first.instance_id, second.instance_id);
        assert_ne!(
            super::super::access::run_generation(&first),
            super::super::access::run_generation(&second)
        );
        let mut legacy = serde_json::to_value(first).unwrap();
        legacy.as_object_mut().unwrap().remove("instanceId");
        assert!(
            serde_json::from_value::<PreviewSessionRecord>(legacy)
                .unwrap()
                .instance_id
                .is_empty()
        );
    }
}
