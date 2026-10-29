//! Reply-owned interactive applications. Services share the workbench preview
//! registry, process manager, logs, and proxy rather than creating another runtime.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::{Mutex, watch},
};

use super::{
    PreviewSessionCreateRequest, PreviewSessionRecord, WorkspacePreviewRegistry,
    WorkspacePreviewRuntime,
};
use crate::server::persistence::db::ServerStateDb;
use crate::{ApiResult, AppError, AppState};

const MAX_HTML: usize = 512 * 1024;
const MAX_STATE: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppService {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: String,
    /// Zero allocates a loopback port; `{port}` in argv is expanded without a shell.
    #[serde(default)]
    port: u16,
    #[serde(default = "default_health_path")]
    health_path: String,
}
fn default_health_path() -> String {
    "/".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppManifest {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    html: String,
    #[serde(default)]
    services: Vec<AppService>,
    #[serde(default)]
    height: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppRecord {
    owner: String,
    directory: String,
    manifest: AppManifest,
    widget_state: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveApp {
    directory: String,
    manifest: AppManifest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSnapshot {
    #[serde(flatten)]
    record: AppRecord,
    services: BTreeMap<String, PreviewSessionRecord>,
}

#[derive(Clone)]
pub(crate) struct InteractiveApps {
    db: Arc<ServerStateDb>,
    registry: Arc<WorkspacePreviewRegistry>,
    runtime: Arc<WorkspacePreviewRuntime>,
    /// Only operations for the same app serialize. Record writes use a separate
    /// short lock so widget state remains writable during HTTP readiness waits.
    operations: Arc<DashMap<String, Weak<AppOperations>>>,
    client: reqwest::Client,
}

struct AppOperations {
    lifecycle: Mutex<()>,
    record: Mutex<()>,
    cancellation: watch::Sender<u64>,
}

impl AppOperations {
    fn new() -> Self {
        Self {
            lifecycle: Mutex::new(()),
            record: Mutex::new(()),
            cancellation: watch::channel(0).0,
        }
    }
}

/// Readiness checks the configured HTTP endpoint. An HTTP error response (for
/// example a missing health route returning 404) is not a ready service.
fn is_http_ready(status: reqwest::StatusCode) -> bool {
    status.is_success() || status.is_redirection()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn app_key(owner: &str, id: &str) -> ApiResult<String> {
    if owner.trim().is_empty() || owner.len() > 200 || !valid_id(id) {
        return Err(AppError::bad_request("invalid interactive app owner or id"));
    }
    Ok(format!(
        "interactiveApp.{}",
        digest(&format!("{owner}\0{id}"))
    ))
}

fn service_id(owner: &str, id: &str, service: &str) -> String {
    format!("app-{}", digest(&format!("{owner}\0{id}\0{service}")))
}

fn digest(value: &str) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_service_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains(['\\', '#', '%'])
        && !path.chars().any(char::is_control)
        && !path
            .split(['/', '?'])
            .any(|part| part == "." || part == "..")
}

impl AppManifest {
    fn validate(&self, id: &str) -> ApiResult<()> {
        if self.id != id
            || !valid_id(id)
            || self.title.trim().is_empty()
            || self.title.len() > 200
            || self.description.len() > 2000
            || self.html.is_empty()
            || self.html.len() > MAX_HTML
            || self.services.len() > 4
            || self.height.is_some_and(|h| !(160..=1200).contains(&h))
        {
            return Err(AppError::bad_request(
                "invalid interactive app manifest or size limits exceeded",
            ));
        }
        let mut names = std::collections::HashSet::new();
        for service in &self.services {
            if !valid_id(&service.name)
                || !names.insert(&service.name)
                || service.command.trim().is_empty()
                || service.command.len() > 4096
                || service.command.contains('\0')
                || service.args.len() > 128
                || service
                    .args
                    .iter()
                    .any(|s| s.len() > 16 * 1024 || s.contains('\0'))
                || service.cwd.len() > 4096
                || !valid_service_path(&service.health_path)
            {
                return Err(AppError::bad_request("invalid interactive app service"));
            }
        }
        Ok(())
    }
}

impl InteractiveApps {
    pub(crate) fn new(
        db: Arc<ServerStateDb>,
        registry: Arc<WorkspacePreviewRegistry>,
        runtime: Arc<WorkspacePreviewRuntime>,
    ) -> Self {
        Self {
            db,
            registry,
            runtime,
            operations: Arc::new(DashMap::new()),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_millis(600))
                .build()
                .expect("loopback readiness client"),
        }
    }

    fn operation(&self, owner: &str, id: &str) -> ApiResult<Arc<AppOperations>> {
        let mut slot = self.operations.entry(app_key(owner, id)?).or_default();
        if let Some(operation) = slot.upgrade() {
            return Ok(operation);
        }
        let operation = Arc::new(AppOperations::new());
        *slot = Arc::downgrade(&operation);
        Ok(operation)
    }

    async fn record(&self, owner: &str, id: &str) -> ApiResult<AppRecord> {
        self.db
            .get_json(&app_key(owner, id)?)
            .await
            .map_err(AppError::internal)?
            .ok_or_else(|| AppError::not_found("interactive app not saved"))
    }

    async fn snapshot(&self, record: AppRecord) -> AppSnapshot {
        let mut services = BTreeMap::new();
        for service in &record.manifest.services {
            if let Some(session) = self
                .registry
                .get_by_id(&service_id(
                    &record.owner,
                    &record.manifest.id,
                    &service.name,
                ))
                .await
            {
                services.insert(service.name.clone(), session);
            }
        }
        AppSnapshot { record, services }
    }

    async fn save(&self, owner: &str, id: &str, request: SaveApp) -> ApiResult<AppSnapshot> {
        let apps = self.clone();
        let owner = owner.to_owned();
        let id = id.to_owned();
        tokio::spawn(async move { apps.save_inner(&owner, &id, request).await })
            .await
            .map_err(|error| {
                AppError::internal(format!("interactive app save task failed: {error}"))
            })?
    }

    async fn save_inner(&self, owner: &str, id: &str, request: SaveApp) -> ApiResult<AppSnapshot> {
        let key = app_key(owner, id)?;
        request.manifest.validate(id)?;
        let operation = self.operation(owner, id)?;
        let _lifecycle = operation.lifecycle.lock().await;
        let _record = operation.record.lock().await;
        let previous = self
            .db
            .get_json::<AppRecord>(&key)
            .await
            .map_err(AppError::internal)?;
        let directory = if request.manifest.services.is_empty() && request.directory.is_empty() {
            String::new()
        } else {
            let path = tokio::fs::canonicalize(&request.directory)
                .await
                .map_err(|_| AppError::bad_request("app directory must exist"))?;
            if !path.is_dir() {
                return Err(AppError::bad_request("app directory must be a folder"));
            }
            path.to_string_lossy().into_owned()
        };
        if let Some(old) = &previous
            && (old.manifest.services != request.manifest.services || old.directory != directory)
        {
            // Updating code while services are active must not silently retarget
            // the proxy or change the command that the user actually ran.
            let mut obsolete = Vec::new();
            for service in &old.manifest.services {
                let sid = service_id(owner, id, &service.name);
                if let Some(session) = self.registry.get_by_id(&sid).await {
                    if session.pid.is_some() {
                        return Err(AppError::conflict(
                            "stop app services before changing their configuration",
                        ));
                    }
                    obsolete.push(sid);
                }
            }
            for sid in obsolete {
                self.runtime.delete_by_id(&sid).await?;
            }
        }
        let record = AppRecord {
            owner: owner.to_owned(),
            directory,
            manifest: request.manifest,
            widget_state: previous.map_or(Value::Null, |old| old.widget_state),
        };
        self.db
            .set_json(&key, record.clone())
            .await
            .map_err(AppError::internal)?;
        Ok(self.snapshot(record).await)
    }

    async fn run(&self, owner: &str, id: &str) -> ApiResult<AppSnapshot> {
        let operation = self.operation(owner, id)?;
        // Subscribe before waiting for the gate so Stop also cancels queued Run
        // requests, instead of accidentally starting them after Stop completes.
        let mut cancellation = operation.cancellation.subscribe();
        let epoch = *cancellation.borrow();
        let _lifecycle = operation.lifecycle.lock().await;
        if *cancellation.borrow() != epoch {
            return Err(AppError::conflict("interactive app run cancelled"));
        }
        let mut started = Vec::new();
        // Cancellation covers loading the record and the final snapshot too,
        // not just each individual HTTP readiness request.
        let result = tokio::select! {
            biased;
            _ = cancellation.changed() => {
                Err(AppError::conflict("interactive app run cancelled"))
            }
            result = async {
                let record = self.record(owner, id).await?;
                let mut runs = Vec::new();
                for service in &record.manifest.services {
                    let run = self.start_service(&record, service, &mut started).await
                        .map_err(|error| AppError::bad_gateway(format!("{}: {error}", service.name)))?;
                    runs.push((service.name.clone(), run));
                }
                // Widget state may have changed while services became ready.
                let snapshot = self.snapshot(self.record(owner, id).await?).await;
                for (name, run) in runs {
                    if !run.is_running() {
                        return Err(AppError::bad_gateway(format!(
                            "{name}: service stopped or was replaced while the app was starting"
                        )));
                    }
                }
                Ok(snapshot)
            } => result,
        };
        let result = if *cancellation.borrow() != epoch {
            Err(AppError::conflict("interactive app run cancelled"))
        } else {
            result
        };
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                for (sid, guard) in started.iter().rev() {
                    if let Err(rollback) = self.runtime.rollback_start(sid, guard).await {
                        tracing::error!(%rollback, session_id = sid, "interactive app rollback failed");
                    }
                }
                return Err(error);
            }
        };
        for (_, guard) in started {
            guard.commit();
        }
        Ok(snapshot)
    }

    async fn start_service(
        &self,
        record: &AppRecord,
        service: &AppService,
        started: &mut Vec<(String, super::runtime::PreviewStartGuard)>,
    ) -> ApiResult<super::runtime::PreviewRun> {
        let sid = service_id(&record.owner, &record.manifest.id, &service.name);
        let (session, run) = if let Some(owned) = self.runtime.owned_session(&sid).await? {
            owned
        } else {
            let session = self.registry.get_by_id(&sid).await;
            if session.is_some() {
                // A stale PID is metadata, never authority to attach to or
                // terminate a process left by an earlier server instance.
                self.runtime.stop_by_id(&sid).await?;
            }
            let base = PathBuf::from(&record.directory);
            let cwd = tokio::fs::canonicalize(base.join(&service.cwd))
                .await
                .map_err(|_| AppError::bad_request("service cwd must exist"))?;
            if !cwd.starts_with(&base) || !cwd.is_dir() {
                return Err(AppError::bad_request(
                    "service cwd must be inside the app directory",
                ));
            }
            // Never attach an app to an unrelated process occupying its port.
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, service.port))
                    .await
                    .map_err(|_| AppError::conflict("service port is already in use"))?;
            let port = listener
                .local_addr()
                .map_err(|err| AppError::internal_error(&err))?
                .port();
            drop(listener);
            if session.is_some() {
                self.runtime.delete_by_id(&sid).await?;
            }
            self.registry
                .create_server_session(PreviewSessionCreateRequest {
                    id: sid.clone(),
                    directory: record.directory.clone(),
                    run_directory: cwd.to_string_lossy().into_owned(),
                    agena_session_id: Some(record.owner.clone()),
                    command: service.command.clone(),
                    args: service
                        .args
                        .iter()
                        .map(|arg| arg.replace("{port}", &port.to_string()))
                        .collect(),
                    logs_path: String::new(),
                    target_url: url::Url::parse(&format!("http://127.0.0.1:{port}/"))
                        .map_err(|e| AppError::bad_request(e.to_string()))?,
                })
                .await?;
            let (running, guard) = self.runtime.start_guarded_by_id(&sid).await?;
            let run = guard.run();
            // If the Run request disappears during readiness, the guard asks
            // the monitor to clean up this exact process generation.
            started.push((sid.clone(), guard));
            (running, run)
        };
        let target = format!(
            "{}{}",
            session.target_url.trim_end_matches('/'),
            service.health_path
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if !run.is_running() {
                return Err(AppError::bad_gateway(
                    "service exited before becoming ready; inspect its logs",
                ));
            }
            if let Ok(response) = self.client.get(&target).send().await
                && is_http_ready(response.status())
                && run.is_running()
            {
                return Ok(run);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::bad_gateway(
                    "service did not become HTTP ready within 15 seconds",
                ));
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    }

    async fn stop(&self, owner: &str, id: &str) -> ApiResult<AppSnapshot> {
        let operation = self.operation(owner, id)?;
        // Signal before acquiring the lifecycle lock: a pending readiness
        // request will roll back promptly and release the gate for this Stop.
        operation
            .cancellation
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        let _lifecycle = operation.lifecycle.lock().await;
        let record = self.record(owner, id).await?;
        for service in &record.manifest.services {
            self.runtime
                .request_stop_by_id(&service_id(owner, id, &service.name));
        }
        let mut error = None;
        for service in record.manifest.services.iter().rev() {
            let sid = service_id(owner, id, &service.name);
            if self.registry.get_by_id(&sid).await.is_some()
                && let Err(err) = self.runtime.stop_by_id(&sid).await
            {
                error = Some(err);
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(self.snapshot(record).await)
    }

    async fn save_state(&self, owner: &str, id: &str, widget_state: Value) -> ApiResult<Value> {
        let apps = self.clone();
        let owner = owner.to_owned();
        let id = id.to_owned();
        tokio::spawn(async move { apps.save_state_inner(&owner, &id, widget_state).await })
            .await
            .map_err(|error| {
                AppError::internal(format!("interactive app state task failed: {error}"))
            })?
    }

    async fn save_state_inner(
        &self,
        owner: &str,
        id: &str,
        widget_state: Value,
    ) -> ApiResult<Value> {
        if serde_json::to_vec(&widget_state)
            .map_err(|err| AppError::internal_error(&err))?
            .len()
            > MAX_STATE
        {
            return Err(AppError::payload_too_large("widget state exceeds 64 KiB"));
        }
        let operation = self.operation(owner, id)?;
        let _record = operation.record.lock().await;
        let mut record = self.record(owner, id).await?;
        record.widget_state = widget_state.clone();
        self.db
            .set_json(&app_key(owner, id)?, record)
            .await
            .map_err(AppError::internal)?;
        Ok(widget_state)
    }
}

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/workbench/apps/{owner}/{id}", get(read).put(save))
        .route("/api/v1/workbench/apps/{owner}/{id}/run", post(run))
        .route("/api/v1/workbench/apps/{owner}/{id}/stop", post(stop))
        .route(
            "/api/v1/workbench/apps/{owner}/{id}/state",
            axum::routing::put(state),
        )
        .route("/api/v1/workbench/apps/{owner}/{id}/logs", get(logs))
}
async fn read(
    State(state): State<Arc<AppState>>,
    Path((owner, id)): Path<(String, String)>,
) -> ApiResult<Json<AppSnapshot>> {
    Ok(Json(
        state
            .interactive_apps
            .snapshot(state.interactive_apps.record(&owner, &id).await?)
            .await,
    ))
}
async fn save(
    State(state): State<Arc<AppState>>,
    Path((owner, id)): Path<(String, String)>,
    Json(body): Json<SaveApp>,
) -> ApiResult<Json<AppSnapshot>> {
    Ok(Json(state.interactive_apps.save(&owner, &id, body).await?))
}
async fn run(
    State(state): State<Arc<AppState>>,
    Path((owner, id)): Path<(String, String)>,
) -> ApiResult<Json<AppSnapshot>> {
    Ok(Json(state.interactive_apps.run(&owner, &id).await?))
}
async fn stop(
    State(state): State<Arc<AppState>>,
    Path((owner, id)): Path<(String, String)>,
) -> ApiResult<Json<AppSnapshot>> {
    Ok(Json(state.interactive_apps.stop(&owner, &id).await?))
}
async fn state(
    State(state): State<Arc<AppState>>,
    Path((owner, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state.interactive_apps.save_state(&owner, &id, body).await?,
    ))
}
async fn logs(
    State(state): State<Arc<AppState>>,
    Path((owner, id)): Path<(String, String)>,
) -> ApiResult<Json<BTreeMap<String, String>>> {
    let snapshot = state
        .interactive_apps
        .snapshot(state.interactive_apps.record(&owner, &id).await?)
        .await;
    let mut logs = BTreeMap::new();
    for (name, session) in snapshot.services {
        logs.insert(name, read_service_logs(&session).await?);
    }
    Ok(Json(logs))
}

async fn read_service_logs(session: &PreviewSessionRecord) -> ApiResult<String> {
    let path = super::runtime::resolve_logs_path(
        &session.logs_path,
        std::path::Path::new(&session.run_directory),
    )?;
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let mut file = match options.open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(AppError::internal_error(&error)),
    };
    let metadata = file
        .metadata()
        .await
        .map_err(|err| AppError::internal_error(&err))?;
    if !metadata.is_file() {
        return Err(AppError::bad_request(
            "logsPath must point to a regular file",
        ));
    }
    file.seek(std::io::SeekFrom::Start(
        metadata.len().saturating_sub(32 * 1024),
    ))
    .await
    .map_err(|err| AppError::internal_error(&err))?;
    let mut bytes = Vec::new();
    file.take(32 * 1024)
        .read_to_end(&mut bytes)
        .await
        .map_err(|err| AppError::internal_error(&err))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::preview::runtime::tests::{TestStore, test_registry};

    #[test]
    fn http_readiness_rejects_client_and_server_error_responses() {
        for status in [200, 204, 301, 302, 307] {
            assert!(is_http_ready(
                reqwest::StatusCode::from_u16(status).unwrap()
            ));
        }
        for status in [100, 401, 403, 404, 429, 500, 503] {
            assert!(!is_http_ready(
                reqwest::StatusCode::from_u16(status).unwrap()
            ));
        }
    }

    fn manifest(id: &str, services: Vec<AppService>) -> AppManifest {
        AppManifest {
            id: id.into(),
            title: id.into(),
            description: String::new(),
            html: "<p>preview lifecycle test</p>".into(),
            services,
            height: None,
        }
    }

    #[tokio::test]
    async fn relative_service_logs_use_the_run_directory_and_keep_the_tail_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let run_directory = directory.path().join("service");
        tokio::fs::create_dir_all(run_directory.join("logs"))
            .await
            .unwrap();
        let content = format!("{}last diagnostic", "x".repeat(40 * 1024));
        tokio::fs::write(run_directory.join("logs/preview.log"), &content)
            .await
            .unwrap();
        let (_, registry) = test_registry(directory.path()).await;
        let session = registry
            .create_server_session(PreviewSessionCreateRequest {
                id: "relative-logs".into(),
                directory: directory.path().to_string_lossy().into_owned(),
                run_directory: run_directory.to_string_lossy().into_owned(),
                agena_session_id: None,
                command: "unused".into(),
                args: vec![],
                logs_path: "logs/preview.log".into(),
                target_url: url::Url::parse("http://127.0.0.1:39999/").unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(
            read_service_logs(&session).await.unwrap(),
            &content[content.len() - 32 * 1024..]
        );
        let mut not_file = session;
        not_file.logs_path = run_directory.to_string_lossy().into_owned();
        assert!(read_service_logs(&not_file).await.is_err());
    }

    #[tokio::test]
    async fn cancelled_manifest_and_state_saves_keep_later_writes_behind_their_commits() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db.clone()));
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry, runtime.clone()));
        apps.save(
            "owner",
            "serialized",
            SaveApp {
                directory: String::new(),
                manifest: manifest("serialized", vec![]),
            },
        )
        .await
        .unwrap();
        let mut changed = manifest("serialized", vec![]);
        changed.html = "<p>new manifest</p>".into();
        store.block_next_write(app_key("owner", "serialized").unwrap());
        let saving = {
            let apps = apps.clone();
            tokio::spawn(async move {
                apps.save(
                    "owner",
                    "serialized",
                    SaveApp {
                        directory: String::new(),
                        manifest: changed,
                    },
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        saving.abort();
        assert!(matches!(saving.await, Err(error) if error.is_cancelled()));
        let operation = apps.operation("owner", "serialized").unwrap();
        assert!(operation.record.try_lock().is_err());
        let state = {
            let apps = apps.clone();
            tokio::spawn(async move {
                apps.save_state("owner", "serialized", serde_json::json!({"revision": 2}))
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!state.is_finished());
        assert_eq!(
            apps.record("owner", "serialized")
                .await
                .unwrap()
                .widget_state,
            Value::Null
        );
        store.resume.notify_one();
        state.await.unwrap().unwrap();
        let record = apps.record("owner", "serialized").await.unwrap();
        assert_eq!(record.manifest.html, "<p>new manifest</p>");
        assert_eq!(record.widget_state, serde_json::json!({"revision": 2}));

        store.block_next_write(app_key("owner", "serialized").unwrap());
        let saving_state = {
            let apps = apps.clone();
            tokio::spawn(async move {
                apps.save_state("owner", "serialized", serde_json::json!({"revision": 3}))
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        saving_state.abort();
        assert!(matches!(saving_state.await, Err(error) if error.is_cancelled()));
        assert!(operation.record.try_lock().is_err());
        let mut changed = manifest("serialized", vec![]);
        changed.html = "<p>latest manifest</p>".into();
        let saving = {
            let apps = apps.clone();
            tokio::spawn(async move {
                apps.save(
                    "owner",
                    "serialized",
                    SaveApp {
                        directory: String::new(),
                        manifest: changed,
                    },
                )
                .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!saving.is_finished());
        assert_eq!(
            apps.record("owner", "serialized")
                .await
                .unwrap()
                .widget_state,
            serde_json::json!({"revision": 2})
        );
        store.resume.notify_one();
        assert!(saving.await.unwrap().is_ok());
        let record = apps.record("owner", "serialized").await.unwrap();
        assert_eq!(record.manifest.html, "<p>latest manifest</p>");
        assert_eq!(record.widget_state, serde_json::json!({"revision": 3}));
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    fn idle_service(name: &str) -> AppService {
        AppService {
            name: name.into(),
            command: "sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            cwd: String::new(),
            port: 0,
            health_path: "/health".into(),
        }
    }

    #[cfg(unix)]
    async fn wait_for_service(
        registry: &WorkspacePreviewRegistry,
        id: &str,
    ) -> PreviewSessionRecord {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(record) = registry.get_by_id(id).await
                    && record.pid.is_some()
                {
                    return record;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("service should have an owned process")
    }

    /// The fixture process is deliberately idle. The test controls exactly
    /// when its HTTP readiness endpoint becomes available.
    #[cfg(unix)]
    async fn serve_health(session: &PreviewSessionRecord) -> tokio::task::JoinHandle<()> {
        let target = url::Url::parse(&session.target_url).unwrap();
        let listener = tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            target.port_or_known_default().unwrap(),
        ))
        .await
        .unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/health", get(|| async { "ready" })),
            )
            .await
            .unwrap();
        })
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_rejects_a_replaced_ready_service_without_stopping_its_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let (db, registry) = test_registry(directory.path()).await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry.clone(), runtime.clone()));
        apps.save(
            "owner",
            "multiple",
            SaveApp {
                directory: directory.path().to_string_lossy().into_owned(),
                manifest: manifest(
                    "multiple",
                    vec![idle_service("first"), idle_service("second")],
                ),
            },
        )
        .await
        .unwrap();
        let running = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.run("owner", "multiple").await })
        };
        let first_id = service_id("owner", "multiple", "first");
        let second_id = service_id("owner", "multiple", "second");
        let first = wait_for_service(&registry, &first_id).await;
        let first_http = serve_health(&first).await;
        let second = wait_for_service(&registry, &second_id).await;
        runtime.stop_by_id(&first_id).await.unwrap();
        first_http.abort();
        let _ = first_http.await;
        let replacement = runtime.start_by_id(&first_id).await.unwrap();
        let second_http = serve_health(&second).await;
        let result = tokio::time::timeout(Duration::from_secs(4), running)
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(result, Err(error) if error.to_string().contains("stopped or was replaced"))
        );
        assert!(
            runtime.is_owned_running(&first_id),
            "rollback must preserve the unrelated replacement launch"
        );
        assert_eq!(
            registry.get_by_id(&first_id).await.unwrap().pid,
            replacement.pid
        );
        assert!(!runtime.is_owned_running(&second_id));
        assert_eq!(registry.get_by_id(&second_id).await.unwrap().pid, None);
        second_http.abort();
        let _ = second_http.await;
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn stop_also_cancels_a_run_waiting_for_its_final_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db.clone()));
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry, runtime));
        apps.save(
            "owner",
            "snapshot",
            SaveApp {
                directory: String::new(),
                manifest: manifest("snapshot", vec![]),
            },
        )
        .await
        .unwrap();
        store.block_read_after(app_key("owner", "snapshot").unwrap(), 1);
        let running = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.run("owner", "snapshot").await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), apps.stop("owner", "snapshot"))
            .await
            .expect("Stop must cancel final snapshot loading and acquire the lifecycle gate")
            .unwrap();
        assert!(
            matches!(running.await.unwrap(), Err(error) if error.to_string().contains("cancelled"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_stop_still_requests_cleanup_of_every_app_service() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore::new(directory.path()));
        let db = Arc::new(ServerStateDb::from_store(store.clone()).await.unwrap());
        let registry = Arc::new(WorkspacePreviewRegistry::new(db.clone()));
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry.clone(), runtime.clone()));
        apps.save(
            "owner",
            "stopping",
            SaveApp {
                directory: directory.path().to_string_lossy().into_owned(),
                manifest: manifest(
                    "stopping",
                    vec![idle_service("first"), idle_service("second")],
                ),
            },
        )
        .await
        .unwrap();
        let running = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.run("owner", "stopping").await })
        };
        let first_id = service_id("owner", "stopping", "first");
        let second_id = service_id("owner", "stopping", "second");
        let first_http = serve_health(&wait_for_service(&registry, &first_id).await).await;
        let second_http = serve_health(&wait_for_service(&registry, &second_id).await).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(4), running)
                .await
                .unwrap()
                .unwrap()
                .is_ok()
        );
        store
            .block_stopped_write
            .store(true, std::sync::atomic::Ordering::Release);
        let stopping = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.stop("owner", "stopping").await })
        };
        tokio::time::timeout(Duration::from_secs(2), store.blocked.notified())
            .await
            .unwrap();
        stopping.abort();
        assert!(matches!(stopping.await, Err(error) if error.is_cancelled()));
        store.resume.notify_one();
        for id in [&first_id, &second_id] {
            tokio::time::timeout(Duration::from_secs(4), async {
                while registry
                    .get_by_id(id)
                    .await
                    .is_some_and(|record| record.pid.is_some())
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("aborting Stop must not leave a later service running");
            assert!(!runtime.is_owned_running(id));
        }
        first_http.abort();
        second_http.abort();
        let _ = tokio::join!(first_http, second_http);
        runtime.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn readiness_does_not_block_other_apps_or_state_and_stop_cancels_the_wait() {
        let directory = tempfile::tempdir().unwrap();
        let (db, registry) = test_registry(directory.path()).await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry.clone(), runtime.clone()));
        apps.save(
            "owner",
            "waiting",
            SaveApp {
                directory: directory.path().to_string_lossy().into_owned(),
                manifest: manifest(
                    "waiting",
                    vec![AppService {
                        name: "server".into(),
                        command: "sh".into(),
                        args: vec!["-c".into(), "exec sleep 30".into()],
                        cwd: String::new(),
                        port: 0,
                        health_path: "/health".into(),
                    }],
                ),
            },
        )
        .await
        .unwrap();
        let running = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.run("owner", "waiting").await })
        };
        let sid = service_id("owner", "waiting", "server");
        tokio::time::timeout(Duration::from_secs(3), async {
            while !runtime.is_owned_running(&sid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !running.is_finished(),
            "a process without an HTTP server is not ready"
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            apps.save(
                "owner",
                "independent",
                SaveApp {
                    directory: String::new(),
                    manifest: manifest("independent", vec![]),
                },
            )
            .await
            .unwrap();
            apps.save_state("owner", "waiting", serde_json::json!({"edited": true}))
                .await
                .unwrap();
        })
        .await
        .expect("unrelated apps and widget state must not wait for readiness");

        tokio::time::timeout(Duration::from_secs(4), apps.stop("owner", "waiting"))
            .await
            .expect("Stop must interrupt the 15-second readiness wait")
            .unwrap();
        assert!(running.await.unwrap().is_err());
        assert!(!runtime.is_owned_running(&sid));
        assert_eq!(registry.get_by_id(&sid).await.unwrap().pid, None);
        assert_eq!(
            apps.record("owner", "waiting").await.unwrap().widget_state,
            serde_json::json!({"edited": true})
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn stop_cancels_a_run_that_is_already_queued_on_the_same_app() {
        let directory = tempfile::tempdir().unwrap();
        let (db, registry) = test_registry(directory.path()).await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry, runtime));
        apps.save(
            "owner",
            "queued",
            SaveApp {
                directory: String::new(),
                manifest: manifest("queued", vec![]),
            },
        )
        .await
        .unwrap();
        let operation = apps.operation("owner", "queued").unwrap();
        let gate = operation.lifecycle.lock().await;
        let running = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.run("owner", "queued").await })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while operation.cancellation.receiver_count() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let stopping = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.stop("owner", "queued").await })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while *operation.cancellation.borrow() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(gate);
        assert!(running.await.unwrap().is_err());
        assert!(stopping.await.unwrap().is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_a_run_during_readiness_cleans_up_its_started_services() {
        let directory = tempfile::tempdir().unwrap();
        let (db, registry) = test_registry(directory.path()).await;
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        let apps = Arc::new(InteractiveApps::new(db, registry.clone(), runtime.clone()));
        apps.save(
            "owner",
            "aborted",
            SaveApp {
                directory: directory.path().to_string_lossy().into_owned(),
                manifest: manifest(
                    "aborted",
                    vec![AppService {
                        name: "server".into(),
                        command: "sh".into(),
                        args: vec!["-c".into(), "exec sleep 30".into()],
                        cwd: String::new(),
                        port: 0,
                        health_path: "/health".into(),
                    }],
                ),
            },
        )
        .await
        .unwrap();
        let running = {
            let apps = apps.clone();
            tokio::spawn(async move { apps.run("owner", "aborted").await })
        };
        let sid = service_id("owner", "aborted", "server");
        tokio::time::timeout(Duration::from_secs(3), async {
            while registry
                .get_by_id(&sid)
                .await
                .is_none_or(|record| record.pid.is_none())
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        running.abort();
        assert!(matches!(running.await, Err(error) if error.is_cancelled()));
        tokio::time::timeout(Duration::from_secs(4), async {
            while registry
                .get_by_id(&sid)
                .await
                .is_some_and(|record| record.pid.is_some())
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancelled readiness must not orphan a successfully spawned service");
        assert!(!runtime.is_owned_running(&sid));
        runtime.shutdown().await.unwrap();
    }
}
