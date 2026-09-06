#![cfg(unix)]

mod support;

use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_api::resource::{
    RuntimeBackgroundTaskListResponse, RuntimeBackgroundTaskStartResponse,
    RuntimeBackgroundTaskStatus,
};
use agena_plugin_sdk::host_api::{
    HostClient, HostConfigReloadState, HostConfigReloadStatusRequest, NoopHostClient,
};
use agena_plugin_sdk::rpc::{
    JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method,
};
use agena_plugin_sdk::{InitContext, InitOutcome, Plugin, PluginManifest};
use reqwest::StatusCode;
use serde_json::{Value, json};

const KEY: &str = "test.application-callback";
const PASSWORD: &str = "isolated-http-plugin-test-password";

#[derive(Default)]
struct Observed {
    initialized: Mutex<Vec<(InitContext, Value)>>,
    host: Mutex<Option<Arc<dyn HostClient>>>,
}

struct CallbackPlugin(Arc<Observed>);

#[async_trait::async_trait]
impl Plugin for CallbackPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test", "application-callback", "1.0.0")
    }

    async fn init(
        &self,
        context: InitContext,
        host: Arc<dyn HostClient>,
    ) -> agena_plugin_sdk::Result<InitOutcome> {
        let config = host.read_config(Some("config.ui.locale".into())).await?;
        self.0.initialized.lock().unwrap().push((context, config));
        *self.0.host.lock().unwrap() = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }
}

struct Processes {
    application: Child,
    plugin: tokio::task::JoinHandle<()>,
    log_path: std::path::PathBuf,
}

impl Drop for Processes {
    fn drop(&mut self) {
        let _ = self.application.kill();
        let _ = self.application.wait();
        self.plugin.abort();
        if std::thread::panicking()
            && let Ok(log) = std::fs::read_to_string(&self.log_path)
        {
            eprintln!("isolated callback application log:\n{log}");
        }
    }
}

#[tokio::test]
async fn authenticated_application_bootstrap_and_reload_serve_plugin_callbacks() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    let state = directory.path().join("state");
    std::fs::create_dir_all(workspace.join(".agena")).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let observed = Arc::new(Observed::default());
    let factory_observed = observed.clone();
    let router = agena_plugin_sdk::drivers::http::router(
        move || CallbackPlugin(factory_observed.clone()),
        Arc::new(NoopHostClient),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plugin_url = format!("http://{}/rpc", listener.local_addr().unwrap());
    let plugin = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let config_path = workspace.join(".agena/agena.json");
    let write_config = |locale| {
        std::fs::write(
            &config_path,
            serde_json::to_vec(&json!({
                "ui": {"locale": locale},
            "plugins": {"list": {KEY: {"package": {"kind": "http", "url": plugin_url}}}}
            }))
            .unwrap(),
        )
        .unwrap();
    };
    write_config("en-US");
    let record_path = state.join("endpoint.json");
    let log_path = directory.path().join("application.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut command = support::isolated_server_command(&state);
    let application = command
        .arg("--database-path")
        .arg(directory.path().join("chat.db"))
        .arg("server")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--workspace")
        .arg(&workspace)
        .env("AGENA_SERVER_UI_PASSWORD", PASSWORD)
        // This fixture owns each reload. A remote startup catalog refresh
        // would rotate callback credentials before the explicit auth checks.
        .env("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES", "1")
        .env("AGENA_SERVER_DATA_DIR", &state)
        .env("AGENA_SERVER_RECORD", &record_path)
        .stdin(Stdio::null())
        .stderr(log.try_clone().unwrap())
        .stdout(log)
        .spawn()
        .unwrap();
    let mut processes = Processes {
        application,
        plugin,
        log_path: log_path.clone(),
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let base_url = loop {
        if let Ok(bytes) = std::fs::read(&record_path)
            && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
            && let Some(url) = record["url"].as_str()
            && let Ok(response) = client.get(format!("{url}/health")).send().await
            && response.status().is_success()
        {
            break url.to_owned();
        }
        assert!(
            processes.application.try_wait().unwrap().is_none()
                && tokio::time::Instant::now() < deadline,
            "isolated application failed to start: {}",
            std::fs::read_to_string(&log_path).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    };

    let (context, initial) = observed
        .initialized
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the production application must initialize a callback-requiring SDK plugin");
    assert_eq!(initial, json!("en-US"));
    assert_eq!(
        client
            .get(format!("{base_url}/api/v1/runtime"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let callback_url = context.host_callback_url.unwrap();
    let bearer = context.host_callback_token.unwrap();
    let request = Request {
        jsonrpc: JsonRpcVersion,
        id: RequestId::Num(71),
        method: method::HOST_CONFIG_READ.into(),
        params: Some(json!({"path": "config.ui.locale", "context": {}})),
        context: None,
    };
    let response = client
        .post(&callback_url)
        .bearer_auth(&bearer)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response: Response = response.json().await.unwrap();
    assert_eq!(response.id, request.id);
    assert!(matches!(response.payload, ResponsePayload::Ok { result } if result == json!("en-US")));

    let login: Value = client
        .post(format!("{base_url}/auth/session"))
        .json(&json!({"password": PASSWORD}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let ui_token = login["token"].as_str().unwrap();
    for token in [None, Some("incorrect-plugin-token"), Some(ui_token)] {
        let mut call = client.post(&callback_url).json(&request);
        if let Some(token) = token {
            call = call.bearer_auth(token);
        }
        let denied = call.send().await.unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let denied: Response = denied.json().await.unwrap();
        assert_eq!(denied.id, request.id);
        assert!(matches!(denied.payload, ResponsePayload::Err { .. }));
    }
    let mut missing_context = request.clone();
    missing_context.params = Some(json!({"path": "config.ui.locale"}));
    let denied = client
        .post(&callback_url)
        .bearer_auth(&bearer)
        .json(&missing_context)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
    assert!(matches!(
        denied.json::<Response>().await.unwrap().payload,
        ResponsePayload::Err { .. }
    ));

    write_config("zh-CN");
    let accepted: RuntimeBackgroundTaskStartResponse = client
        .post(format!("{base_url}/api/v1/runtime/reload"))
        .bearer_auth(ui_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(accepted.started);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            // Candidate configuration and callback credentials can become
            // visible before publication binds the runtime services. Wait for
            // this accepted task to finish before requesting another reload.
            let tasks: RuntimeBackgroundTaskListResponse = client
                .get(format!("{base_url}/api/v1/runtime/tasks"))
                .bearer_auth(ui_token)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let task = tasks
                .items
                .iter()
                .find(|task| task.id == accepted.task.id)
                .expect("accepted application reload remains in task history");
            match task.status {
                RuntimeBackgroundTaskStatus::Running => {}
                RuntimeBackgroundTaskStatus::Succeeded => break,
                _ => panic!("application REST reload failed: {task:?}"),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("accepted REST reload reaches a real terminal success");
    let host = observed.host.lock().unwrap().clone().unwrap();
    assert_eq!(
        host.read_config(Some("config.ui.locale".into()))
            .await
            .unwrap(),
        json!("zh-CN")
    );
    assert_eq!(
        observed.initialized.lock().unwrap().len(),
        1,
        "unchanged plugin rebinds without another init"
    );
    assert_eq!(
        client
            .post(callback_url)
            .bearer_auth(bearer)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    // A background SDK callback also uses the dedicated listener, independent
    // of the UI session. Acceptance and completion have different wire shapes.
    write_config("en-US");
    let accepted = host.request_config_reload().await.unwrap();
    assert!(accepted.started);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            // A request sent with the previous binding may be rejected
            // while the successor installs the new URL/token pair.
            if let Ok(status) = host
                .config_reload_status(HostConfigReloadStatusRequest {
                    task_id: accepted.task_id.clone(),
                })
                .await
            {
                match status.state {
                    HostConfigReloadState::Running {} => {}
                    HostConfigReloadState::Succeeded {} => break,
                    terminal => panic!("application self reload failed: {terminal:?}"),
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("accepted callback reload reaches a real terminal success");
    assert_eq!(
        host.read_config(Some("config.ui.locale".into()))
            .await
            .unwrap(),
        json!("en-US")
    );
    assert_eq!(observed.initialized.lock().unwrap().len(), 1);
}
