//! Exercise the public HTTP routes against the backend installed for HostClient.
#![cfg(feature = "http")]

use agena_api::{commands::Command, queries::Query};
use agena_api_server::{AppState, router};
use agena_application::Application;
use agena_notification::model::{
    ActionTarget, Notification, NotificationAction, NotificationKind, NotificationScope,
    NotificationSeverity, NotificationSource, NotificationSurface,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    application: Application,
    runtime: agena_runtime::RuntimeBootstrapResult,
    router: Router,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("workspace")).unwrap();
        let plugins = agena_bundled_plugins::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .map(|id| (id, json!({"enabled": false, "package": {"kind": "static"}})))
            .collect::<serde_json::Map<String, Value>>();
        std::fs::write(
            root.join("agena.json"),
            serde_json::to_vec(&json!({"plugins": {"list": plugins}})).unwrap(),
        )
        .unwrap();
        let (application, runtime) =
            Application::bootstrap(agena_runtime::RuntimeBootstrapRequest {
                workspace_root: Some(root.join("workspace")),
                config_path: Some(root.join("agena.json")),
                database_path: Some(root.join("test.sqlite")),
                automatic_maintenance: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        let router = router(AppState::from_application(application.clone()));
        Self {
            application,
            runtime,
            router,
            _directory: directory,
        }
    }

    async fn http(&self, method: Method, path: &str, input: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        let body = if let Some(input) = input {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&input).unwrap())
        } else {
            Body::empty()
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or_else(|error| {
                panic!(
                    "invalid HTTP JSON ({status}): {error}; {}",
                    String::from_utf8_lossy(&body)
                )
            })
        };
        (status, value)
    }

    async fn query(&self, request: Value) -> Value {
        let backend = self
            .runtime
            .application_services()
            .session_api_backend
            .backend()
            .unwrap();
        let query: Query = serde_json::from_value(request.clone()).unwrap();
        let expected = backend.query(query).await.unwrap();
        let (status, actual) = self
            .http(Method::POST, "/api/v1/queries", Some(request))
            .await;
        assert_eq!(status, StatusCode::OK, "{actual}");
        assert_eq!(
            actual, expected,
            "HTTP and Host backend must return the same resource envelope"
        );
        actual
    }

    async fn command(&self, request: Value) -> Value {
        let backend = self
            .runtime
            .application_services()
            .session_api_backend
            .backend()
            .unwrap();
        let command: Command = serde_json::from_value(request).unwrap();
        backend.command(command).await.unwrap()
    }

    async fn close(self) {
        self.runtime.shutdown_and_wait().await.unwrap();
    }

    async fn conditional_workspaces(&self, etag: Option<&str>) -> (StatusCode, String, Value) {
        self.conditional_get("/api/v1/workspaces", etag).await
    }

    async fn conditional_get(&self, path: &str, etag: Option<&str>) -> (StatusCode, String, Value) {
        let mut request = Request::builder().uri(path);
        if let Some(etag) = etag {
            request = request.header("if-none-match", etag);
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let etag = response
            .headers()
            .get("etag")
            .expect("conditional resource reads carry a revision")
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let data = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (status, etag, data)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

fn business(method: &str, params: Value) -> Value {
    json!({"method": "business", "params": {"method": method, "params": params}})
}

fn extended_session(method: &str, params: Value) -> Value {
    json!({"method": "extended_session", "params": {"method": method, "params": params}})
}

#[tokio::test]
async fn host_workspace_writes_invalidate_http_etags_without_spurious_resolve_changes() {
    let fixture = Fixture::new().await;
    let first_path = fixture._directory.path().join("catalog-first");
    let next_path = fixture._directory.path().join("catalog-next");
    std::fs::create_dir(&first_path).unwrap();
    std::fs::create_dir(&next_path).unwrap();
    let (status, before, _) = fixture.conditional_workspaces(None).await;
    assert_eq!(status, StatusCode::OK);
    let created = fixture
        .command(json!({
            "method": "create_workspace", "params": {"path": first_path}
        }))
        .await;
    let id = created["data"]["id"].as_i64().unwrap();
    let (status, after_create, workspaces) = fixture.conditional_workspaces(Some(&before)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a Host create must invalidate the HTTP catalog revision"
    );
    assert_ne!(before, after_create);
    assert!(
        workspaces["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|workspace| workspace["id"] == id)
    );

    let resolved = fixture
        .command(json!({
            "method": "resolve_workspace", "params": {"path": first_path, "create_if_missing": true}
        }))
        .await;
    assert_eq!(resolved["data"]["id"], id);
    let (status, after_resolve, _) = fixture.conditional_workspaces(Some(&after_create)).await;
    assert_eq!(
        status,
        StatusCode::NOT_MODIFIED,
        "resolving an existing workspace is read-only"
    );
    assert_eq!(after_resolve, after_create);

    fixture
        .command(json!({"method": "update_workspace", "params": {
            "workspace_id": id, "path": next_path, "expected_version": created["data"]["version"]
        }}))
        .await;
    let (status, after_update, workspaces) =
        fixture.conditional_workspaces(Some(&after_create)).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(after_update, after_create);
    let changed = workspaces["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|workspace| workspace["id"] == id)
        .unwrap();
    assert_eq!(changed["path"], json!(next_path));
    fixture
        .command(json!({"method": "delete_workspace", "params": {"workspace_id": id}}))
        .await;
    let (status, after_delete, workspaces) =
        fixture.conditional_workspaces(Some(&after_update)).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(after_delete, after_update);
    assert!(
        !workspaces["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|workspace| workspace["id"] == id)
    );
    fixture.close().await;
}

#[tokio::test]
async fn http_and_host_backend_share_bot_state_revisions_and_owned_conversations() {
    let fixture = Fixture::new().await;
    let create = json!({
        "request_key": "operation-parity-create", "name": "Parity Bot",
        "persona": {"instructions": "Keep source evidence."}
    });
    let created = fixture
        .command(business("create_bot", create.clone()))
        .await;
    let id = created["data"]["id"].as_str().unwrap();

    // Replaying one creation through either HTTP entry preserves its identity.
    let (status, replay) = fixture
        .http(
            Method::POST,
            "/api/v1/commands",
            Some(business("create_bot", create.clone())),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay, created);
    let (status, legacy) = fixture
        .http(Method::POST, "/api/v1/bots", Some(create))
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy, created["data"]);

    let listed = fixture.query(business("list_bots", json!({}))).await;
    let (status, legacy) = fixture.http(Method::GET, "/api/v1/bots", None).await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy, listed["data"]);
    assert!(
        legacy["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|bot| bot["id"] == id)
    );

    let update = json!({
        "request_key": "operation-parity-rename",
        "expected_revision": created["data"]["revision"],
        "name": "Renamed Parity Bot", "persona": {"instructions": "Explain the evidence."}
    });
    let path = format!("/api/v1/bots/{id}");
    let (status, renamed) = fixture.http(Method::PUT, &path, Some(update)).await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    let read = fixture
        .query(business("get_bot", json!({"bot_id": id})))
        .await;
    assert_eq!(read["data"], renamed);
    assert_eq!(read["data"]["name"], "Renamed Parity Bot");

    let stale = business(
        "update_bot",
        json!({
            "bot_id": id, "request_key": "operation-parity-stale",
            "expected_revision": created["data"]["revision"],
            "name": "Stale overwrite", "persona": {"instructions": ""}
        }),
    );
    let backend = fixture
        .runtime
        .application_services()
        .session_api_backend
        .backend()
        .unwrap();
    let failure = backend
        .command(serde_json::from_value(stale.clone()).unwrap())
        .await
        .unwrap_err();
    assert_eq!(failure.category, agena_failure::FailureCategory::Conflict);
    let (status, rejected) = fixture
        .http(Method::POST, "/api/v1/commands", Some(stale))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
    assert_eq!(
        rejected["problem"]["category"],
        serde_json::to_value(failure.category).unwrap()
    );
    assert_eq!(rejected["problem"]["code"], failure.code.as_str());

    let owner_path = format!("/api/v1/sessions?owner_bot_id={id}");
    let surface_path = "/api/v1/sessions?owner_surface=local";
    let (status, before_create, _) = fixture.conditional_get(&owner_path, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, surface_before_create, _) = fixture.conditional_get(surface_path, None).await;
    assert_eq!(status, StatusCode::OK);
    let original_local = created["data"]["local_session_id"].as_i64().unwrap();
    let reused = fixture
        .command(business(
            "create_bot_conversation",
            json!({
                "bot_id": id, "title": "A retry must not rename the local conversation"
            }),
        ))
        .await;
    assert_eq!(reused["data"]["session_id"], original_local);
    let (status, legacy_reused) = fixture
        .http(
            Method::POST,
            &format!("/api/v1/bots/{id}/sessions"),
            Some(json!({"title": "Another ignored retry title"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy_reused}");
    assert_eq!(legacy_reused, reused["data"]);
    let (status, unchanged_owner, _) = fixture
        .conditional_get(&owner_path, Some(&before_create))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_MODIFIED,
        "reopening must not rename or duplicate the local session"
    );
    assert_eq!(unchanged_owner, before_create);
    fixture
        .command(json!({"method":"delete_session","params":{"session_id":original_local}}))
        .await;
    let without_local = fixture
        .query(business("get_bot", json!({"bot_id": id})))
        .await;
    assert!(without_local["data"]["local_session_id"].is_null());
    let (status, conversation) = fixture
        .http(
            Method::POST,
            "/api/v1/commands",
            Some(business(
                "create_bot_conversation",
                json!({"bot_id": id, "title": "HTTP conversation"}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{conversation}");
    assert_eq!(conversation["data"]["bot_id"], id);
    let session_id = conversation["data"]["session_id"].as_i64().unwrap();
    assert_ne!(
        session_id, original_local,
        "deletion must create a fresh local conversation"
    );
    let owned = fixture
        .query(json!({
            "method": "list_sessions", "params": {"owner_bot_id": id}
        }))
        .await;
    assert!(
        owned["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|session| session["id"] == session_id),
        "the Bot's global conversation must survive its explicit owner filter: {owned}"
    );
    assert!(
        owned["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|session| session["owner_bot_id"] == id)
    );
    let (status, after_create, legacy_owned) = fixture
        .conditional_get(&owner_path, Some(&before_create))
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy_owned}");
    assert_ne!(before_create, after_create);
    assert_eq!(legacy_owned, owned["data"]);
    let (status, unchanged, _) = fixture
        .conditional_get(&owner_path, Some(&after_create))
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(unchanged, after_create);
    let by_surface = fixture
        .query(json!({
            "method": "list_sessions", "params": {"owner_surface": "local"}
        }))
        .await;
    assert!(
        by_surface["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|session| session["id"] == session_id)
    );
    let (status, surface_after_create, legacy_surface) = fixture
        .conditional_get(surface_path, Some(&surface_before_create))
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy_surface}");
    assert_ne!(surface_before_create, surface_after_create);
    assert_eq!(legacy_surface, by_surface["data"]);
    let (status, _, _) = fixture
        .conditional_get(surface_path, Some(&surface_after_create))
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);

    fixture
        .command(extended_session(
            "update_metadata",
            json!({
                "session_id": session_id, "title": "Changed through the backend"
            }),
        ))
        .await;
    let (status, after_update, updated) = fixture
        .conditional_get(&owner_path, Some(&after_create))
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_ne!(after_update, after_create);
    let changed = updated["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == session_id)
        .unwrap();
    assert_eq!(changed["title"], "Changed through the backend");

    let parent_path = format!("/api/v1/sessions?parent_id={session_id}");
    let (status, before_child, _) = fixture.conditional_get(&parent_path, None).await;
    assert_eq!(status, StatusCode::OK);
    // Seed a global child through the same durable kernel without starting a run.
    let kernel = fixture.application.session_kernel().unwrap();
    let parent: agena_storage::kernel::SessionId = session_id.try_into().unwrap();
    let owner = kernel
        .reader
        .configuration(parent)
        .await
        .unwrap()
        .owner
        .clone();
    let child = kernel
        .sessions
        .create_global(
            agena_storage::kernel::RequestId::new(),
            "Global child".into(),
            agena_storage::kernel::SessionConfiguration {
                owner,
                parent_session_id: Some(parent),
                relation_kind: agena_domain::SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id
        .get();
    let (status, after_child, children) = fixture
        .conditional_get(&parent_path, Some(&before_child))
        .await;
    assert_eq!(status, StatusCode::OK, "{children}");
    assert_ne!(before_child, after_child);
    assert!(
        children["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|session| session["id"] == child)
    );
    let (status, _, _) = fixture
        .conditional_get(&parent_path, Some(&after_child))
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    let ensured = fixture
        .command(business("ensure_bot_session", json!({"bot_id": id})))
        .await;
    let (status, legacy_ensured) = fixture
        .http(Method::POST, &format!("/api/v1/bots/{id}/session"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy_ensured}");
    assert_eq!(legacy_ensured, ensured["data"]);

    fixture.close().await;
}

#[tokio::test]
async fn http_and_host_backend_share_notification_filters_actions_and_dismissals() {
    let fixture = Fixture::new().await;
    fixture.application.notifications().ingest(Notification {
        id: "operation-parity-notification".into(),
        kind: NotificationKind::Notice {
            code: "operation-parity".into(),
        },
        severity: NotificationSeverity::Info,
        scope: NotificationScope::Plugin("operation-parity".into()),
        surface: NotificationSurface::Log,
        source: NotificationSource::Plugin,
        summary: "A shared notification".into(),
        detail: None,
        control: Default::default(),
        actions: vec![NotificationAction {
            id: "copy".into(),
            label: "Copy".into(),
            target: ActionTarget::Copy {
                text: "shared result".into(),
            },
        }],
        priority: 0,
        dedup_key: None,
        created_at_ms: 100,
        expires_at_ms: None,
        dismissed: false,
    });
    let filter = json!({"scope_kind": "plugin", "scope_key": "operation-parity"});
    let listed = fixture
        .query(business("list_notifications", filter.clone()))
        .await;
    assert_eq!(listed["data"]["items"].as_array().unwrap().len(), 1);
    let legacy_path = "/api/v1/notifications?scope_kind=plugin&scope_key=operation-parity";
    let (status, legacy) = fixture.http(Method::GET, legacy_path, None).await;
    assert_eq!(status, StatusCode::OK, "{legacy}");
    assert_eq!(legacy, listed["data"]);

    let resolved = fixture
        .command(business(
            "resolve_notification_action",
            json!({
                "notification_id": "operation-parity-notification", "action_id": "copy"
            }),
        ))
        .await;
    let (status, legacy_target) = fixture
        .http(
            Method::POST,
            "/api/v1/notifications/operation-parity-notification/actions/copy",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy_target}");
    assert_eq!(legacy_target, resolved["data"]);
    assert_eq!(
        legacy_target,
        json!({"target": "copy", "text": "shared result"})
    );

    let (status, dismissed) = fixture
        .http(
            Method::POST,
            "/api/v1/commands",
            Some(business(
                "dismiss_notification",
                json!({"notification_id": "operation-parity-notification"}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dismissed}");
    let active = fixture.query(business("list_notifications", filter)).await;
    assert_eq!(active["data"]["items"], json!([]));
    let all = fixture
        .query(business(
            "list_notifications",
            json!({
                "scope_kind": "plugin", "scope_key": "operation-parity", "active_only": false
            }),
        ))
        .await;
    assert_eq!(all["data"]["items"][0]["dismissed"], true);
    let (status, legacy_all) = fixture
        .http(
            Method::GET,
            &format!("{legacy_path}&active_only=false"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{legacy_all}");
    assert_eq!(legacy_all, all["data"]);

    let invalid = business("list_notifications", json!({"scope_kind": "session"}));
    let backend = fixture
        .runtime
        .application_services()
        .session_api_backend
        .backend()
        .unwrap();
    let failure = backend
        .query(serde_json::from_value(invalid.clone()).unwrap())
        .await
        .unwrap_err();
    assert_eq!(
        failure.category,
        agena_failure::FailureCategory::InvalidInput
    );
    let (status, rejected) = fixture
        .http(Method::POST, "/api/v1/queries", Some(invalid))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["problem"]["code"], failure.code.as_str());

    fixture.close().await;
}

#[test]
fn chunked_attachment_retries_publish_one_file_across_transports() {
    // Managed attachment paths have process scope. Give the child its own data
    // root without mutating this process's environment or other test fixtures.
    const CHILD: &str = "AGENA_OPERATION_PARITY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "chunked_attachment_retries_publish_one_file_across_transports",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("AGENA_DATA_DIR", directory.path().join("managed"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "isolated attachment test failed: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let fixture = Fixture::new().await;
            let created = fixture
                .command(business(
                    "create_bot",
                    json!({
                        "request_key": "attachment-parity-bot", "name": "Attachment Bot"
                    }),
                ))
                .await;
            let session = fixture
                .command(business(
                    "ensure_bot_session",
                    json!({
                        "bot_id": created["data"]["id"]
                    }),
                ))
                .await;
            let session_id = session["data"]["session_id"].as_i64().unwrap();
            let upload = fixture
                .command(extended_session(
                    "begin_upload",
                    json!({
                        "kind": "attachment", "session_id": session_id,
                        "filename": "evidence.txt", "mime": "text/plain"
                    }),
                ))
                .await;
            let transfer_id = upload["data"]["transfer_id"].clone();
            let first = extended_session(
                "write_transfer",
                json!({
                    "transfer_id": transfer_id, "offset": 0, "data_base64": "YWJj"
                }),
            );
            let (status, written) = fixture
                .http(Method::POST, "/api/v1/commands", Some(first.clone()))
                .await;
            assert_eq!(status, StatusCode::OK, "{written}");
            assert_eq!(written["data"]["next_offset"], 3);
            assert_eq!(fixture.command(first).await, written);
            let appended = fixture
                .command(extended_session(
                    "write_transfer",
                    json!({
                        "transfer_id": transfer_id, "offset": 3, "data_base64": "ZGVm"
                    }),
                ))
                .await;
            assert_eq!(appended["data"]["next_offset"], 6);
            let finish = extended_session("finish_upload", json!({"transfer_id": transfer_id}));
            let completed = fixture.command(finish.clone()).await;
            let (status, replay) = fixture
                .http(Method::POST, "/api/v1/commands", Some(finish))
                .await;
            assert_eq!(status, StatusCode::OK, "{replay}");
            assert_eq!(
                replay, completed,
                "retrying completion must return the original committed file"
            );
            assert_eq!(completed["data"]["size_bytes"], 6);
            let path = std::path::Path::new(completed["data"]["path"].as_str().unwrap());
            let data_root = std::path::PathBuf::from(std::env::var_os("AGENA_DATA_DIR").unwrap());
            assert!(path.starts_with(data_root));
            assert_eq!(std::fs::read(path).unwrap(), b"abcdef");
            assert_eq!(
                std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
                1
            );

            let media = fixture
                .command(extended_session(
                    "open_media",
                    json!({
                        "session_id": session_id, "path": path
                    }),
                ))
                .await;
            let download_id = media["data"]["transfer_id"].clone();
            let chunk = fixture
                .query(extended_session(
                    "read_transfer",
                    json!({
                        "transfer_id": download_id, "offset": 0
                    }),
                ))
                .await;
            assert_eq!(chunk["data"]["data_base64"], "YWJjZGVm");
            assert_eq!(chunk["data"]["eof"], true);
            for id in [transfer_id, download_id] {
                fixture
                    .command(extended_session(
                        "close_transfer",
                        json!({"transfer_id": id}),
                    ))
                    .await;
            }
            fixture.close().await;
        });
}
