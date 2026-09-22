use agena_domain::{StructuredObject, ToolInvocation};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_runtime_tools::{
    authorization::ExecutionPrincipal,
    permission::{PermissionPolicy, ToolPermissionPolicy},
    tool::{ToolError, ToolExecutor},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{OriginalUri, State},
    http::HeaderMap,
};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
#[derive(Clone, Debug)]
pub struct Captured {
    pub method: String,
    pub raw: Vec<u8>,
    pub authenticated: bool,
    pub path: String,
    pub body: Value,
    pub beta: Option<String>,
}
#[derive(Default)]
pub struct ServerState {
    pub requests: Mutex<Vec<Captured>>,
    pub response: Mutex<Option<Value>>,
    pub status: Mutex<Option<u16>>,
    pub upload_target: Mutex<Option<String>>,
    pub deleted: Mutex<std::collections::BTreeSet<String>>,
    pub block_upload: std::sync::atomic::AtomicBool,
    pub upload_entered: tokio::sync::Notify,
}
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub executor: ToolExecutor,
    pub state: Arc<ServerState>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    pub async fn new() -> Self {
        assert_eq!(
            std::env::var("AGENA_HOSTED_TEST_KEY").as_deref(),
            Ok("synthetic-provider-test-only"),
            "run the HTTP fixture in an isolated test process"
        );
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::write(
            workspace.join("fixture.png"),
            base64::engine::general_purpose::STANDARD
                .decode(PNG)
                .unwrap(),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(ServerState::default());
        let app = Router::new().fallback(respond).with_state(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut config = PluginsConfig::default();
        for (name, base) in [
            ("chatgpt", format!("{url}/openai/v1")),
            ("claude", format!("{url}/anthropic")),
            ("gemini", format!("{url}/google/v1beta")),
        ] {
            let mut settings = json!({"base_url":base,"api_key_env":"AGENA_HOSTED_TEST_KEY","model":"fixture-model","timeout_secs":3});
            if name == "chatgpt" {
                settings["cache_mode"] = json!("disabled");
                settings["image_model"] = json!("fixture-image");
            }
            if name == "gemini" {
                settings["image_model"] = json!("fixture-image");
            }
            config.list.insert(
                format!("agena.{name}"),
                ConfiguredPlugin {
                    settings,
                    ..ConfiguredPlugin::static_default()
                },
            );
        }
        for name in ["fs", "shell"] {
            config
                .list
                .insert(format!("agena.{name}"), ConfiguredPlugin::static_default());
        }
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    "agena.chatgpt".parse().unwrap(),
                    agena_bundled_plugins::tool::new_chatgpt_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.claude".parse().unwrap(),
                    agena_bundled_plugins::tool::new_claude_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.gemini".parse().unwrap(),
                    agena_bundled_plugins::tool::new_gemini_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.fs".parse().unwrap(),
                    agena_bundled_plugins::tool::new_fs_plugin(),
                ),
                StaticPluginRegistration::new(
                    "agena.shell".parse().unwrap(),
                    agena_bundled_plugins::tool::new_shell_plugin(),
                ),
            ],
            config,
            workspace_root: workspace.clone(),
            agena_version: "hosted-test".into(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: HashMap::new(),
        })
        .await
        .unwrap();
        Self {
            dir,
            executor: ToolExecutor::new(
                workspace,
                ExecutionPrincipal::new(
                    PermissionPolicy::allow_all(),
                    ToolPermissionPolicy::allow_all(),
                ),
                plugins,
                None,
                None,
                None,
            ),
            state,
            server,
        }
    }
    pub async fn call(&self, name: &str, input: Value) -> Result<(Value, String), ToolError> {
        self.call_as(name, input, 41, 1).await
    }
    pub async fn call_as(
        &self,
        name: &str,
        input: Value,
        session: i64,
        id: i64,
    ) -> Result<(Value, String), ToolError> {
        let invocation = ToolInvocation::new(name, StructuredObject::try_from(input).unwrap());
        let prepared = self
            .executor
            .prepare_invocation(&invocation, session, id)
            .await?;
        let result = self
            .executor
            .execute_invocation_detailed(&prepared.invocation, session, id)
            .await?;
        Ok((Value::from(result.output.payload), result.view.output_text))
    }
    pub fn requests(&self) -> Vec<Captured> {
        self.state.requests.lock().unwrap().clone()
    }
}
async fn respond(
    State(state): State<Arc<ServerState>>,
    OriginalUri(uri): OriginalUri,
    method: axum::http::Method,
    headers: HeaderMap,
    bytes: Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let body = if content_type.starts_with("multipart/form-data") {
        json!({"multipart":true})
    } else {
        serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null)
    };
    let path = uri.path().to_string();
    let auth = headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
        .or_else(|| headers.get("x-goog-api-key"))
        .and_then(|v| v.to_str().ok());
    if path.ends_with("/upload/session") {
        assert!(
            auth.is_none(),
            "do not forward API credentials to resumable URL"
        );
    } else {
        assert!(matches!(
            auth,
            Some("synthetic-provider-test-only" | "Bearer synthetic-provider-test-only")
        ));
    }
    state.requests.lock().unwrap().push(Captured {
        path: path.clone(),
        body: body.clone(),
        method: method.to_string(),
        raw: bytes.to_vec(),
        authenticated: auth.is_some(),
        beta: headers
            .get("anthropic-beta")
            .map(|v| v.to_str().unwrap().into()),
    });
    if method == axum::http::Method::POST
        && path.ends_with("/files")
        && state.block_upload.load(std::sync::atomic::Ordering::SeqCst)
    {
        state.upload_entered.notify_one();
        std::future::pending::<()>().await;
    }
    if let Some(status) = *state.status.lock().unwrap() {
        return (
            axum::http::StatusCode::from_u16(status).unwrap(),
            Json(json!({"error":"injected"})),
        )
            .into_response();
    }
    if let Some(response) = state.response.lock().unwrap().clone() {
        return Json(response).into_response();
    }
    let origin = format!("http://{}", headers.get("host").unwrap().to_str().unwrap());
    if path.ends_with("/upload/v1beta/files") {
        let target = state
            .upload_target
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| format!("{origin}/google/upload/session"));
        return ([("x-goog-upload-url", target)], Json(json!({}))).into_response();
    }
    if path.ends_with("/upload/session") || path.ends_with("/files") || path.contains("/files/") {
        let google = path.starts_with("/google/");
        let remote = if google {
            "files/file_fixture"
        } else {
            "file_fixture"
        };
        if method == axum::http::Method::DELETE {
            state.deleted.lock().unwrap().insert(path.clone());
            return Json(json!({"id":remote,"deleted":true})).into_response();
        }
        if method == axum::http::Method::GET && state.deleted.lock().unwrap().contains(&path) {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        }
        let result = if google {
            json!({"name":remote,"uri":format!("{origin}/google/v1beta/{remote}"),"state":"ACTIVE","expirationTime":"2099-01-01T00:00:00Z"})
        } else if path.starts_with("/anthropic/") {
            json!({"id":remote,"type":"file","filename":"fixture.png","mime_type":"image/png","size_bytes":1,"expires_at":"2099-01-01T00:00:00Z"})
        } else {
            json!({"id":remote,"status":"processed","expires_at":4070908800_i64})
        };
        return Json(if path.ends_with("/upload/session") {
            json!({"file":result})
        } else {
            result
        })
        .into_response();
    }
    if body.get("tools").is_none() && !path.contains("/images/") {
        if path.ends_with("/messages") {
            return Json(json!({"id":"msg_media","stop_reason":"end_turn","content":[{"type":"text","text":"Image/document content understood."}]})).into_response();
        }
        if path.ends_with(":generateContent")
            && body["generationConfig"]["responseModalities"].is_null()
        {
            return Json(json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"Image/document content understood."}]}}]})).into_response();
        }
        if path.ends_with("/responses") {
            return Json(json!({"id":"resp_media","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Image/document content understood."}]}]})).into_response();
        }
    }
    let tool = body
        .pointer("/tools/0/type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if path.contains("/images/") {
        return Json(json!({"data":[{"b64_json":PNG}]})).into_response();
    }
    if path.ends_with(":generateContent") {
        return Json(
            json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{"inlineData":{"mimeType":"image/png","data":PNG}}]}}]}),
        ).into_response();
    }
    if path.ends_with("/messages") {
        let name = body
            .pointer("/tools/0/name")
            .and_then(Value::as_str)
            .unwrap();
        let result_name = match name {
            "code_execution" => "bash_code_execution_tool_result",
            "advisor" => "advisor_tool_result",
            "web_fetch" => "web_fetch_tool_result",
            _ => "web_search_tool_result",
        };
        return Json(
            json!({"id":"msg_fixture","stop_reason":"end_turn","content":[{"type":"server_tool_use","id":"srv_fixture","name":name,"input":{"query":"fixture"}},{"type":result_name,"tool_use_id":"srv_fixture","content":[]},{"type":"text","text":"Hosted fixture complete."}],"usage":{"input_tokens":1,"output_tokens":2}}),
        ).into_response();
    }
    if path.ends_with("/interactions") {
        return Json(
            json!({"id":"int_fixture","status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":"Hosted fixture complete."}]}]}),
        ).into_response();
    }
    let output = if tool == "shell" {
        json!([
            {"type":"shell_call","call_id":"call_fixture","status":"completed","action":{"commands":["printf cloud-only"]}},
            {"type":"shell_call_output","call_id":"call_fixture","output":[{"stdout":"cloud-only","outcome":{"type":"exit","exit_code":0}}]}
        ])
    } else if tool == "image_generation" {
        json!([{"type":"image_generation_call","status":"completed","result":PNG}])
    } else {
        json!([{"type":format!("{tool}_call"),"status":"completed"},{"type":"message","content":[{"type":"output_text","text":"Hosted fixture complete."}]}])
    };
    Json(
        json!({"id":"resp_fixture","status":"completed","output":output,"usage":{"input_tokens":1,"output_tokens":2}}),
    ).into_response()
}
pub fn isolate(name: &str) -> bool {
    if std::env::var("AGENA_HOSTED_TEST_CHILD").as_deref() == Ok(name) {
        return false;
    }
    let home = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("AGENA_HOSTED_TEST_CHILD", name)
        .env("AGENA_HOSTED_TEST_KEY", "synthetic-provider-test-only")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "isolated hosted test failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    true
}
