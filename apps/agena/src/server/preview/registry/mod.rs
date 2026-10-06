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

#[derive(Debug)]
pub(crate) struct WorkspacePreviewRegistry {
    db: Arc<crate::server::persistence::db::ServerStateDb>,
    ttl: Duration,
    cache: RwLock<Option<RegistryCache>>,
    epoch: uuid::Uuid,
    changes: tokio::sync::watch::Sender<u64>,
    events: tokio::sync::Mutex<Option<PreviewEventSnapshot>>,
}

impl WorkspacePreviewRegistry {
    pub(crate) fn new(db: Arc<crate::server::persistence::db::ServerStateDb>) -> Self {
        Self::new_with_ttl(db, CACHE_TTL)
    }

    fn new_with_ttl(db: Arc<crate::server::persistence::db::ServerStateDb>, ttl: Duration) -> Self {
        Self {
            db,
            ttl,
            cache: RwLock::new(None),
            epoch: uuid::Uuid::new_v4(),
            changes: tokio::sync::watch::channel(0).0,
            events: tokio::sync::Mutex::new(None),
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
    ) -> ApiResult<Option<PreviewSessionRecord>> {
        let mut file = self.load_server_state_file().await?;
        let updated = mark_stopped_in_file(&mut file, id, error, updated_at)?;
        if updated.is_some() {
            self.write_server_state_file(&file).await?;
        }
        Ok(updated)
    }

    pub(crate) async fn create_server_session(
        &self,
        request: PreviewSessionCreateRequest,
    ) -> ApiResult<PreviewSessionRecord> {
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

        let args = args
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();
        if args.is_empty() {
            return Err(AppError::bad_request("args is required"));
        }

        let agena_session_id = agena_session_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let record = PreviewSessionRecord {
            id: trimmed_id.to_string(),
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

    pub(crate) async fn delete_by_id(&self, id: &str) -> ApiResult<()> {
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

    pub(crate) async fn update_by_id(
        &self,
        id: &str,
        patch: PreviewSessionUpdatePatch,
    ) -> ApiResult<PreviewSessionRecord> {
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

    pub(crate) async fn rename_by_id(
        &self,
        id: &str,
        new_id: &str,
    ) -> ApiResult<PreviewSessionRecord> {
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

    pub(crate) async fn mark_running_by_id(
        &self,
        id: &str,
        pid: u32,
    ) -> ApiResult<PreviewSessionRecord> {
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

    pub(crate) async fn mark_stopped_by_id(
        &self,
        id: &str,
        error: Option<String>,
    ) -> ApiResult<PreviewSessionRecord> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("id is required"));
        }

        let updated_at = now_millis();

        if let Some(updated) = self
            .mark_stopped_in_server_store(trimmed, error.clone(), updated_at)
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
        let next = next
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>();
        if next.is_empty() {
            return Err(AppError::bad_request("args is required"));
        }
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
) -> ApiResult<Option<PreviewSessionRecord>> {
    let Some(index) = file.sessions.iter().position(|session| session.id == id) else {
        return Ok(None);
    };

    let mut record = file.sessions[index].clone();
    record.state = "stopped".to_string();
    record.pid = None;
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
        if session.args.is_empty() {
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
mod preview_delta_tests {
    use super::*;

    fn record(id: &str, directory: &str) -> PreviewSessionRecord {
        serde_json::from_value(serde_json::json!({
            "id": id, "directory": directory, "runDirectory": directory, "state": "running",
            "proxyBasePath": format!("/api/v1/workbench/preview/s/{id}/"), "targetUrl": "http://localhost:3000",
            "command": "bun", "args": ["dev"], "logsPath": "/tmp/preview.log", "updatedAt": 1,
        })).unwrap()
    }

    #[tokio::test]
    async fn preview_delta_failed_reads_do_not_cache_an_empty_registry() {
        let directory = tempfile::tempdir().unwrap();
        let db = Arc::new(
            db::ServerStateDb::open_at_path(directory.path().join("state.sqlite"))
                .await
                .unwrap(),
        );
        db.set_json(
            db::KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE,
            serde_json::json!({"invalid":"snapshot"}),
        )
        .await
        .unwrap();
        let registry = WorkspacePreviewRegistry::new(db.clone());
        assert!(registry.list_all().await.is_err());
        assert!(registry.event_payload(None).await.is_err());
        let revision = registry.revision();
        db.set_json(
            db::KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE,
            PreviewSessionsFile {
                updated_at: 1,
                sessions: vec![record("A", "/repo-A")],
            },
        )
        .await
        .unwrap();
        let (recovered_revision, payload) = registry.event_payload(None).await.unwrap();
        assert_eq!(
            recovered_revision, revision,
            "storage recovery requires no unrelated mutation"
        );
        let recovered: serde_json::Value = serde_json::from_str(&payload.unwrap()).unwrap();
        assert_eq!(recovered["delta"]["reset"], true);
        assert_eq!(recovered["delta"]["upsert"][0]["id"], "A");
        assert_eq!(registry.list_all().await.unwrap().sessions.len(), 1);
    }

    #[test]
    fn preview_delta_transmits_only_changed_records_and_deleted_ids() {
        let a = record("A", "/repo-A");
        let b = record("B", "/repo-B");
        let old = PreviewSessionsResponse {
            updated_at: 1,
            sessions: vec![a.clone(), b.clone()],
        };
        let mut changed = a;
        changed.state = "stopped".into();
        changed.updated_at = 2;
        let next = PreviewSessionsResponse {
            updated_at: 2,
            sessions: vec![changed, b.clone()],
        };
        let delta = preview_session_delta(Some(&old), &next);
        assert_eq!(delta["reset"], false);
        assert_eq!(delta["upsert"].as_array().unwrap().len(), 1);
        assert_eq!(delta["upsert"][0]["id"], "A");
        assert!(delta["removed"].as_array().unwrap().is_empty());
        let renamed = PreviewSessionsResponse {
            updated_at: 3,
            sessions: vec![record("renamed-A", "/repo-A"), b],
        };
        let delta = preview_session_delta(Some(&next), &renamed);
        assert_eq!(delta["removed"], serde_json::json!(["A"]));
        assert_eq!(delta["upsert"][0]["id"], "renamed-A");
        assert_eq!(delta["upsert"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn preview_delta_reconnect_replaces_the_snapshot_and_idle_is_empty() {
        let snapshot = PreviewSessionsResponse {
            updated_at: 1,
            sessions: vec![record("A", "/repo-A"), record("B", "/repo-B")],
        };
        let initial = preview_session_delta(None, &snapshot);
        assert_eq!(initial["reset"], true);
        assert_eq!(initial["upsert"].as_array().unwrap().len(), 2);
        let idle = preview_session_delta(Some(&snapshot), &snapshot);
        assert_eq!(idle["reset"], false);
        assert!(idle["upsert"].as_array().unwrap().is_empty());
        assert!(idle["removed"].as_array().unwrap().is_empty());
    }

    #[test]
    fn preview_delta_shares_serialized_frames_and_resets_consumers_that_missed_a_revision() {
        let initial = PreviewEventSnapshot::new(
            "preview:1".into(),
            None,
            PreviewSessionsResponse {
                updated_at: 1,
                sessions: vec![record("A", "/repo-A"), record("B", "/repo-B")],
            },
        );
        let mut changed = record("A", "/repo-A");
        changed.state = "stopped".into();
        let mut next = PreviewEventSnapshot::new(
            "preview:2".into(),
            Some(&initial),
            PreviewSessionsResponse {
                updated_at: 2,
                sessions: vec![changed.clone(), record("B", "/repo-B")],
            },
        );
        let first = next.payload(Some("preview:1")).unwrap();
        let second = next.payload(Some("preview:1")).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "all browsers reuse one serialized delta"
        );
        let delta: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(delta["delta"]["reset"], false);
        assert_eq!(delta["delta"]["upsert"].as_array().unwrap().len(), 1);
        assert_eq!(delta["delta"]["upsert"][0]["id"], "A");
        assert!(next.payload(Some("preview:2")).is_none());

        let mut newest = PreviewEventSnapshot::new(
            "preview:3".into(),
            Some(&next),
            PreviewSessionsResponse {
                updated_at: 3,
                sessions: vec![changed, record("B", "/repo-B")],
            },
        );
        let reset = newest.payload(Some("preview:1")).unwrap();
        let fresh = newest.payload(None).unwrap();
        assert!(
            Arc::ptr_eq(&reset, &fresh),
            "slow and new browsers reuse the same reset"
        );
        let delta: serde_json::Value = serde_json::from_str(&reset).unwrap();
        assert_eq!(delta["revision"], "preview:3");
        assert_eq!(delta["delta"]["reset"], true);
        assert_eq!(delta["delta"]["upsert"].as_array().unwrap().len(), 2);
        assert!(newest.payload(Some("preview:3")).is_none());
    }
}
