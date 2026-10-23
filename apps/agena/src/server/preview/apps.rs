//! Reply-owned interactive applications. Services share the workbench preview
//! registry, process manager, logs, and proxy rather than creating another runtime.
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::Mutex,
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

pub(crate) struct InteractiveApps {
    db: Arc<ServerStateDb>,
    registry: Arc<WorkspacePreviewRegistry>,
    runtime: Arc<WorkspacePreviewRuntime>,
    /// Serializes read/modify/write and service lifecycles, including repeated
    /// clicks from different panes. Parsing a reply never starts a process.
    operations: Mutex<()>,
    client: reqwest::Client,
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
            operations: Mutex::new(()),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_millis(600))
                .build()
                .expect("loopback readiness client"),
        }
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
        let key = app_key(owner, id)?;
        request.manifest.validate(id)?;
        let _operation = self.operations.lock().await;
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
                self.registry.delete_by_id(&sid).await?;
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
        let _operation = self.operations.lock().await;
        let record = self.record(owner, id).await?;
        let mut started = Vec::new();
        for service in &record.manifest.services {
            let result = self.start_service(&record, service, &mut started).await;
            if let Err(error) = result {
                for sid in started.iter().rev() {
                    if let Err(rollback) = self.runtime.stop_by_id(sid).await {
                        tracing::error!(%rollback, session_id = sid, "interactive app rollback failed");
                    }
                }
                return Err(AppError::bad_gateway(format!("{}: {error}", service.name)));
            }
        }
        Ok(self.snapshot(record).await)
    }

    async fn start_service(
        &self,
        record: &AppRecord,
        service: &AppService,
        started: &mut Vec<String>,
    ) -> ApiResult<()> {
        let sid = service_id(&record.owner, &record.manifest.id, &service.name);
        let mut session = self.registry.get_by_id(&sid).await;
        if session.as_ref().is_none_or(|s| s.pid.is_none()) {
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
                self.registry.delete_by_id(&sid).await?;
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
            session = Some(self.runtime.start_by_id(&sid).await?);
            started.push(sid.clone());
        }
        let session = session.ok_or_else(|| AppError::internal("missing app preview service"))?;
        let target = format!(
            "{}{}",
            session.target_url.trim_end_matches('/'),
            service.health_path
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let running = self.registry.get_by_id(&sid).await;
            if running.as_ref().and_then(|s| s.pid).is_none() {
                return Err(AppError::bad_gateway(
                    "service exited before becoming ready; inspect its logs",
                ));
            }
            if let Ok(response) = self.client.get(&target).send().await
                && response.status().as_u16() < 500
            {
                return Ok(());
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
        let _operation = self.operations.lock().await;
        let record = self.record(owner, id).await?;
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
        if serde_json::to_vec(&widget_state)
            .map_err(|err| AppError::internal_error(&err))?
            .len()
            > MAX_STATE
        {
            return Err(AppError::payload_too_large("widget state exceeds 64 KiB"));
        }
        let _operation = self.operations.lock().await;
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
        let mut file = match tokio::fs::File::open(&session.logs_path).await {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                logs.insert(name, String::new());
                continue;
            }
            Err(error) => return Err(AppError::internal_error(&error)),
        };
        let len = file
            .metadata()
            .await
            .map_err(|err| AppError::internal_error(&err))?
            .len();
        file.seek(std::io::SeekFrom::Start(len.saturating_sub(32 * 1024)))
            .await
            .map_err(|err| AppError::internal_error(&err))?;
        let mut bytes = Vec::new();
        file.take(32 * 1024)
            .read_to_end(&mut bytes)
            .await
            .map_err(|err| AppError::internal_error(&err))?;
        logs.insert(name, String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(Json(logs))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn manager(
        path: &std::path::Path,
    ) -> (agena_storage_sqlite::SqliteKernel, InteractiveApps) {
        let kernel = agena_storage_sqlite::open_kernel_runtime(&format!(
            "sqlite://{}?mode=rwc",
            path.join("agena.db").display()
        ))
        .await
        .unwrap();
        let db = Arc::new(
            ServerStateDb::from_store(kernel.instance_state.clone())
                .await
                .unwrap(),
        );
        let registry = Arc::new(WorkspacePreviewRegistry::new(db.clone()));
        let runtime = Arc::new(WorkspacePreviewRuntime::new(registry.clone()));
        (kernel, InteractiveApps::new(db, registry, runtime))
    }

    fn manifest(path: &std::path::Path) -> AppManifest {
        let server = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/interactive-app/server.py")
            .canonicalize()
            .unwrap();
        AppManifest {
            id: "tasks".into(),
            title: "Tasks".into(),
            description: String::new(),
            html: "<main>Tasks</main>".into(),
            height: Some(480),
            services: vec![AppService {
                name: "api".into(),
                command: "python3".into(),
                args: vec![
                    server.to_string_lossy().into_owned(),
                    "--port".into(),
                    "{port}".into(),
                    "--database".into(),
                    path.join("tasks.sqlite").to_string_lossy().into_owned(),
                ],
                cwd: String::new(),
                port: 0,
                health_path: "/health".into(),
            }],
        }
    }

    #[test]
    fn manifest_and_keys_are_bounded_and_scoped() {
        assert_ne!(
            app_key("one", "tasks").unwrap(),
            app_key("two", "tasks").unwrap()
        );
        assert!(app_key("session", "../escape").is_err());
        for path in [
            "//evil.test",
            "/../other",
            "/%2e%2e/other",
            "/back\\slash",
            "/foo#fragment",
        ] {
            assert!(!valid_service_path(path));
        }
        assert!(valid_service_path("/health?ready=1"));
        let mut app = AppManifest {
            id: "demo".into(),
            title: "Demo".into(),
            description: String::new(),
            html: "<main/>".into(),
            services: vec![],
            height: None,
        };
        assert!(app.validate("demo").is_ok());
        app.html = "x".repeat(MAX_HTML + 1);
        assert!(app.validate("demo").is_err());
    }

    #[tokio::test]
    async fn real_backend_lifecycle_is_idempotent_and_preserves_data_and_widget_state() {
        let temp = tempfile::tempdir().unwrap();
        let (kernel, apps) = manager(temp.path()).await;
        let app = manifest(temp.path());
        let request = SaveApp {
            directory: temp.path().to_string_lossy().into_owned(),
            manifest: app,
        };
        let saved = apps.save("session", "tasks", request).await.unwrap();
        assert!(
            saved.services.is_empty(),
            "saving a reply must not execute code"
        );
        let (first, second) =
            tokio::join!(apps.run("session", "tasks"), apps.run("session", "tasks"));
        let first = first.unwrap();
        assert_eq!(
            first.services["api"].pid,
            second.unwrap().services["api"].pid
        );
        let target = first.services["api"].target_url.clone();
        let client = reqwest::Client::new();
        let created: Value = client
            .post(format!("{target}tasks"))
            .json(&serde_json::json!({"title":"durable task"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = created["id"].as_u64().unwrap();
        let response = client
            .patch(format!("{target}tasks/{id}"))
            .json(&serde_json::json!({"done":true}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        apps.save_state("session", "tasks", serde_json::json!({"filter":"done"}))
            .await
            .unwrap();
        assert!(
            apps.save_state("other-session", "tasks", Value::Null)
                .await
                .is_err()
        );
        let stopped = apps.stop("session", "tasks").await.unwrap();
        assert!(stopped.services["api"].pid.is_none());
        let restarted = apps.run("session", "tasks").await.unwrap();
        let result: Value = client
            .get(format!("{}tasks", restarted.services["api"].target_url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        apps.stop("session", "tasks").await.unwrap();
        assert!(
            result["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|task| task["id"] == id && task["done"] == true)
        );
        drop(apps);
        kernel.ports.clone().shutdown().await.unwrap();
        drop(kernel);
        let (_reopened_kernel, reloaded) = manager(temp.path()).await;
        assert_eq!(
            reloaded
                .record("session", "tasks")
                .await
                .unwrap()
                .widget_state["filter"],
            "done"
        );
        assert!(
            tokio::fs::metadata(&restarted.services["api"].logs_path)
                .await
                .unwrap()
                .len()
                > 0
        );
    }

    #[tokio::test]
    async fn partial_start_rolls_back_new_services_and_rejects_workspace_escape() {
        let temp = tempfile::tempdir().unwrap();
        let (_kernel, apps) = manager(temp.path()).await;
        let mut app = manifest(temp.path());
        let mut invalid = app.services[0].clone();
        invalid.name = "broken".into();
        invalid.command = "/agena-missing-command".into();
        app.services.push(invalid);
        apps.save(
            "session",
            "tasks",
            SaveApp {
                directory: temp.path().to_string_lossy().into_owned(),
                manifest: app,
            },
        )
        .await
        .unwrap();
        assert!(apps.run("session", "tasks").await.is_err());
        let snapshot = apps
            .snapshot(apps.record("session", "tasks").await.unwrap())
            .await;
        assert!(
            snapshot.services.values().all(|s| s.pid.is_none()),
            "rollback must not leave an API process running"
        );
        let mut app = manifest(temp.path());
        app.services[0].cwd = "..".into();
        apps.save(
            "session",
            "tasks",
            SaveApp {
                directory: temp.path().to_string_lossy().into_owned(),
                manifest: app,
            },
        )
        .await
        .unwrap();
        assert!(apps.run("session", "tasks").await.is_err());
    }

    #[tokio::test]
    async fn rejected_configuration_change_keeps_every_existing_service() {
        let temp = tempfile::tempdir().unwrap();
        let (_kernel, apps) = manager(temp.path()).await;
        let mut app = manifest(temp.path());
        let mut second = app.services[0].clone();
        second.name = "second".into();
        app.services.push(second);
        apps.save(
            "session",
            "tasks",
            SaveApp {
                directory: temp.path().to_string_lossy().into_owned(),
                manifest: app.clone(),
            },
        )
        .await
        .unwrap();
        for service in &app.services {
            apps.registry
                .create_server_session(PreviewSessionCreateRequest {
                    id: service_id("session", "tasks", &service.name),
                    directory: temp.path().to_string_lossy().into_owned(),
                    run_directory: temp.path().to_string_lossy().into_owned(),
                    agena_session_id: Some("session".into()),
                    command: service.command.clone(),
                    args: service.args.clone(),
                    logs_path: String::new(),
                    target_url: url::Url::parse("http://127.0.0.1:3000/").unwrap(),
                })
                .await
                .unwrap();
        }
        // Only a registry marker, no process is spawned by this test.
        apps.registry
            .mark_running_by_id(&service_id("session", "tasks", "second"), 123)
            .await
            .unwrap();
        app.services.clear();
        assert!(
            apps.save(
                "session",
                "tasks",
                SaveApp {
                    directory: temp.path().to_string_lossy().into_owned(),
                    manifest: app
                }
            )
            .await
            .is_err()
        );
        let snapshot = apps
            .snapshot(apps.record("session", "tasks").await.unwrap())
            .await;
        assert_eq!(
            snapshot.services.len(),
            2,
            "a rejected update must not delete earlier stopped services"
        );
    }
}
