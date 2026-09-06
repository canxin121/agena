#![cfg(feature = "http")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agena_plugin_sdk::drivers::http::{
    self, EXPECTED_REVISION_HEADER, HttpInstanceState, INSTANCE_HEADER,
};
use agena_plugin_sdk::host_api::{HostClient, NoopHostClient};
use agena_plugin_sdk::rpc::{
    JsonRpcVersion, Request, RequestId, Response, ResponsePayload, method,
};
use agena_plugin_sdk::{
    InitContext, InitOutcome, Plugin, PluginError, PluginSettings, ToolInvokeInput,
    ToolInvokeOutput, agena_plugin,
};
use serde_json::{Value, json};

#[derive(Default)]
struct Observed {
    created: AtomicUsize,
    shutdowns: Mutex<Vec<usize>>,
    dropped: Mutex<Vec<usize>>,
}

#[derive(Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Settings {
    revision: usize,
    #[serde(default)]
    fail: bool,
    #[serde(default)]
    bad_outcome: bool,
    #[serde(default)]
    bad_manifest: bool,
}

struct SettingsPlugin {
    id: usize,
    settings: PluginSettings<Settings>,
    observed: Arc<Observed>,
}

impl SettingsPlugin {
    fn new(observed: Arc<Observed>) -> Self {
        Self {
            id: observed.created.fetch_add(1, Ordering::SeqCst) + 1,
            settings: PluginSettings::new(),
            observed,
        }
    }
}

impl Drop for SettingsPlugin {
    fn drop(&mut self) {
        self.observed
            .dropped
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(self.id);
    }
}

#[agena_plugin(
    namespace = "test",
    name = "http-settings",
    version = "1.0.0",
    summary = "Exercise HTTP macro plugin object lifetimes.",
    settings = Settings,
    settings_default = default,
    settings_field = settings,
)]
impl SettingsPlugin {
    #[hook(init)]
    async fn initialize(
        &self,
        _: InitContext,
        _: Arc<dyn HostClient>,
    ) -> agena_plugin_sdk::Result<InitOutcome> {
        if self.settings.expect("settings initialized by macro").fail {
            return Err(PluginError::internal("injected initialization failure"));
        }
        let mut outcome = InitOutcome::ack(self.manifest());
        if self
            .settings
            .expect("settings initialized by macro")
            .bad_outcome
        {
            outcome.protocol_version += 1;
        }
        if self
            .settings
            .expect("settings initialized by macro")
            .bad_manifest
        {
            outcome.manifest.version = "2.0.0".into();
        }
        Ok(outcome)
    }

    #[hook(shutdown)]
    async fn shutdown_instance(&self) -> agena_plugin_sdk::Result<()> {
        self.observed.shutdowns.lock().unwrap().push(self.id);
        Ok(())
    }

    #[tool(summary = "Read the current object and settings.", read_only)]
    fn snapshot(&self) -> ToolInvokeOutput {
        ToolInvokeOutput::text(format!(
            "{}:{}",
            self.id,
            self.settings
                .expect("settings initialized by macro")
                .revision,
        ))
    }
}

struct Fixture {
    client: reqwest::Client,
    url: String,
    server: tokio::task::JoinHandle<()>,
    observed: Arc<Observed>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let observed = Arc::new(Observed::default());
        let factory_observed = observed.clone();
        let router = http::router(
            move || SettingsPlugin::new(factory_observed.clone()),
            Arc::new(NoopHostClient),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/rpc", listener.local_addr().unwrap());
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            url,
            server: tokio::spawn(async move { axum::serve(listener, router).await.unwrap() }),
            observed,
        }
    }

    async fn call(
        &self,
        owner: &str,
        expected: Option<&str>,
        name: &str,
        params: Value,
    ) -> Response {
        let mut request = self
            .client
            .post(&self.url)
            .header(INSTANCE_HEADER, owner)
            .json(&Request {
                jsonrpc: JsonRpcVersion,
                id: RequestId::Num(1),
                method: name.into(),
                params: Some(params),
                context: None,
            });
        if let Some(expected) = expected {
            request = request.header(EXPECTED_REVISION_HEADER, expected);
        }
        let response = request
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Response>()
            .await
            .unwrap();
        assert_eq!(response.id, RequestId::Num(1));
        response
    }

    async fn init(&self, owner: &str, revision: usize, fail: bool) -> Response {
        self.init_with_settings(owner, json!({"revision": revision, "fail": fail}))
            .await
    }

    async fn init_with_settings(&self, owner: &str, settings: Value) -> Response {
        let state: HttpInstanceState = serde_json::from_value(success(
            self.call(owner, None, method::META_HTTP_STATE, json!({}))
                .await,
        ))
        .unwrap();
        self.call(
            owner,
            Some(state.revision.as_deref().unwrap_or("-")),
            method::META_INIT,
            serde_json::to_value(InitContext {
                agena_version: "test".into(),
                plugin_id: "test.http-settings".parse().unwrap(),
                workspace_root: "/test".into(),
                host_callback_url: None,
                host_callback_token: None,
                settings,
                protocol_version: agena_plugin_sdk::rpc::PROTOCOL_VERSION,
            })
            .unwrap(),
        )
        .await
    }

    async fn snapshot(&self, owner: &str) -> String {
        let response = self
            .call(
                owner,
                None,
                method::HOOK_TOOL_INVOKE,
                serde_json::to_value(ToolInvokeInput {
                    tool_name: "snapshot".into(),
                    session_id: 1,
                    call_id: 1,
                    workspace_root: "/test".into(),
                    input: json!({}),
                })
                .unwrap(),
            )
            .await;
        serde_json::from_value::<ToolInvokeOutput>(success(response))
            .unwrap()
            .output_text
    }
}

fn success(response: Response) -> Value {
    match response.payload {
        ResponsePayload::Ok { result } => result,
        ResponsePayload::Err { error } => panic!("HTTP plugin call failed: {error:?}"),
    }
}

#[tokio::test]
async fn macro_plugin_settings_reload_constructs_a_new_object() {
    let fixture = Fixture::new().await;
    success(fixture.init("first", 10, false).await);
    assert_eq!(fixture.snapshot("first").await, "1:10");
    success(fixture.init("second", 20, false).await);
    assert_eq!(fixture.snapshot("second").await, "2:20");
    assert_eq!(*fixture.observed.shutdowns.lock().unwrap(), vec![1]);
    assert!(fixture.observed.dropped.lock().unwrap().contains(&1));
    assert!(matches!(
        fixture
            .call("first", None, method::META_SHUTDOWN, json!({}))
            .await
            .payload,
        ResponsePayload::Err { .. }
    ));
    assert_eq!(fixture.snapshot("second").await, "2:20");
}

#[tokio::test]
async fn failed_macro_initialization_can_retry_with_fresh_settings() {
    let fixture = Fixture::new().await;
    assert!(matches!(
        fixture.init("first", 10, true).await.payload,
        ResponsePayload::Err { .. }
    ));
    success(fixture.init("second", 20, false).await);
    assert_eq!(fixture.snapshot("second").await, "2:20");
    assert_eq!(*fixture.observed.shutdowns.lock().unwrap(), vec![1]);
    assert!(fixture.observed.dropped.lock().unwrap().contains(&1));
}

#[tokio::test]
async fn successful_shutdown_releases_the_plugin_object_while_the_router_stays_available() {
    let fixture = Fixture::new().await;
    success(fixture.init("first", 10, false).await);
    success(
        fixture
            .call("first", None, method::META_SHUTDOWN, json!({}))
            .await,
    );
    assert_eq!(*fixture.observed.shutdowns.lock().unwrap(), vec![1]);
    assert_eq!(*fixture.observed.dropped.lock().unwrap(), vec![1]);
    success(
        fixture
            .call("first", None, method::META_SHUTDOWN, json!({}))
            .await,
    );
    assert_eq!(*fixture.observed.shutdowns.lock().unwrap(), vec![1]);
    success(fixture.init("second", 20, false).await);
    assert_eq!(fixture.snapshot("second").await, "2:20");
}

#[tokio::test]
async fn incompatible_init_outcome_is_cleaned_without_admitting_plugin_calls() {
    for field in ["bad_outcome", "bad_manifest"] {
        let fixture = Fixture::new().await;
        let mut settings = json!({"revision": 10});
        settings[field] = json!(true);
        let response = fixture.init_with_settings("first", settings).await;
        assert!(matches!(response.payload, ResponsePayload::Err { .. }));
        let response = fixture
            .call(
                "first",
                None,
                method::HOOK_TOOL_INVOKE,
                serde_json::to_value(ToolInvokeInput {
                    tool_name: "snapshot".into(),
                    session_id: 1,
                    call_id: 1,
                    workspace_root: "/test".into(),
                    input: json!({}),
                })
                .unwrap(),
            )
            .await;
        assert!(matches!(response.payload, ResponsePayload::Err { .. }));
        success(fixture.init("second", 20, false).await);
        assert_eq!(fixture.snapshot("second").await, "2:20");
        assert_eq!(*fixture.observed.shutdowns.lock().unwrap(), vec![1]);
        assert!(fixture.observed.dropped.lock().unwrap().contains(&1));
    }
}

#[tokio::test]
async fn callback_rebind_preserves_the_current_macro_plugin_object_and_settings() {
    let fixture = Fixture::new().await;
    success(fixture.init("first", 10, false).await);
    success(
        fixture
            .call(
                "first",
                None,
                method::META_HOST_REBIND,
                json!({"expected": {}, "next": {}}),
            )
            .await,
    );
    assert_eq!(fixture.snapshot("first").await, "1:10");
    assert_eq!(fixture.observed.created.load(Ordering::SeqCst), 1);
    assert!(fixture.observed.shutdowns.lock().unwrap().is_empty());
    assert!(fixture.observed.dropped.lock().unwrap().is_empty());
}
