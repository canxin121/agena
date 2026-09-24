use std::sync::atomic::AtomicUsize;

use tokio::sync::Notify;

use super::*;
use crate::host_api::NoopHostClient;
use crate::rpc::RequestId;
use crate::{InitOutcome, PluginManifest, ToolStreamEnd, ToolStreamSink};

const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
    dropped: AtomicBool,
}

struct Dropped<'a>(&'a AtomicBool);
impl Drop for Dropped<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct CountDrop<'a>(&'a AtomicUsize);
impl Drop for CountDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Observed {
    inits: AtomicUsize,
    shutdowns: AtomicUsize,
    calls: AtomicUsize,
    streams_started: AtomicUsize,
    streams_finished: AtomicUsize,
    init: Gate,
    invoke: Gate,
    stream: Gate,
    shutdown: Gate,
    hosts: Mutex<Vec<Arc<dyn HostClient>>>,
    fail_shutdown: AtomicBool,
    block_shutdown: AtomicBool,
    factory_panics: AtomicBool,
    manifest_drifts: AtomicBool,
}

struct TestPlugin(Arc<Observed>);

#[async_trait::async_trait]
impl Plugin for TestPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(
            "test",
            "instance",
            if self.0.manifest_drifts.load(Ordering::SeqCst) {
                "2.0.0"
            } else {
                "1.0.0"
            },
        )
    }
    async fn init(
        &self,
        ctx: InitContext,
        host: Arc<dyn HostClient>,
    ) -> crate::Result<InitOutcome> {
        self.0.inits.fetch_add(1, Ordering::SeqCst);
        self.0.hosts.lock().unwrap().push(host);
        if ctx.settings["block"] == true {
            let _dropped = Dropped(&self.0.init.dropped);
            self.0.init.entered.notify_one();
            self.0.init.release.notified().await;
        }
        if ctx.settings["fail"] == true {
            return Err(PluginError::internal("injected init failure"));
        }
        Ok(InitOutcome::ack(self.manifest()))
    }
    async fn shutdown(&self) -> crate::Result<()> {
        self.0.shutdowns.fetch_add(1, Ordering::SeqCst);
        self.0.shutdown.entered.notify_one();
        if self.0.block_shutdown.load(Ordering::SeqCst) {
            let _dropped = Dropped(&self.0.shutdown.dropped);
            self.0.shutdown.release.notified().await;
        }
        if self.0.fail_shutdown.load(Ordering::SeqCst) {
            return Err(PluginError::internal("injected shutdown failure"));
        }
        Ok(())
    }
    async fn tool_invoke(&self, input: ToolInvokeInput) -> crate::Result<ToolInvokeOutput> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        if input.input["block"] == true {
            let _dropped = Dropped(&self.0.invoke.dropped);
            self.0.invoke.entered.notify_one();
            self.0.invoke.release.notified().await;
        }
        Ok(ToolInvokeOutput::text("alive"))
    }
    async fn tool_invoke_stream(
        &self,
        _: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> crate::Result<ToolStreamEnd> {
        let _dropped = Dropped(&self.0.stream.dropped);
        let _count = CountDrop(&self.0.streams_finished);
        self.0.streams_started.fetch_add(1, Ordering::SeqCst);
        self.0.stream.entered.notify_one();
        self.0.stream.release.notified().await;
        Ok(ToolStreamEnd::text(sink.stream_id(), "complete"))
    }
}

struct Fixture {
    state: Arc<HttpDriverState<TestPlugin>>,
    observed: Arc<Observed>,
}

struct WorkingHost;

#[async_trait::async_trait]
impl HostClient for WorkingHost {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, event: EventEnvelope) -> crate::Result<()> {
        NoopHostClient.publish_event(event).await
    }
    async fn subscribe_events(&self, filter: EventFilter) -> crate::Result<EventSubscription> {
        NoopHostClient.subscribe_events(filter).await
    }
    async fn read_config(&self, _: Option<String>) -> crate::Result<Value> {
        Ok(serde_json::json!({"host": "working"}))
    }
    async fn invoke_tool(&self, tool: String, input: Value) -> crate::Result<ToolInvokeOutput> {
        NoopHostClient.invoke_tool(tool, input).await
    }
}

impl Fixture {
    fn new() -> Self {
        let observed = Arc::new(Observed::default());
        let fallback: Arc<dyn HostClient> = Arc::new(WorkingHost);
        Self {
            observed: observed.clone(),
            state: Arc::new(HttpDriverState::new(
                move || {
                    assert!(
                        !observed.factory_panics.load(Ordering::SeqCst),
                        "injected factory panic"
                    );
                    TestPlugin(observed.clone())
                },
                fallback,
            )),
        }
    }

    async fn call(&self, headers: HeaderMap, method_name: &str, params: Value) -> Response {
        tokio::time::timeout(
            WAIT,
            handle_rpc(
                State(self.state.clone()),
                headers,
                Json(Request {
                    jsonrpc: JsonRpcVersion,
                    id: RequestId::Num(1),
                    method: method_name.into(),
                    params: Some(params),
                    context: None,
                }),
            ),
        )
        .await
        .unwrap()
        .0
    }

    async fn revision(&self) -> Option<String> {
        let response = success(
            self.call(
                HeaderMap::new(),
                method::META_HTTP_STATE,
                serde_json::json!({}),
            )
            .await,
        );
        serde_json::from_value::<HttpInstanceState>(response)
            .unwrap()
            .revision
    }

    async fn init(
        &self,
        owner: Option<&str>,
        expected: Option<&str>,
        settings: Value,
        callbacks: bool,
    ) -> Response {
        let mut headers = owner_headers(owner);
        if let Some(expected) = expected {
            headers.insert(EXPECTED_REVISION_HEADER, expected.parse().unwrap());
        }
        self.call(headers, method::META_INIT, init_params(settings, callbacks))
            .await
    }

    async fn invoke(&self, owner: Option<&str>, blocked: bool, stream: bool) -> Response {
        self.call(
            owner_headers(owner),
            if stream {
                method::HOOK_TOOL_INVOKE_STREAM
            } else {
                method::HOOK_TOOL_INVOKE
            },
            serde_json::to_value(ToolInvokeInput {
                tool_name: "test".into(),
                session_id: 1,
                call_id: 1,
                workspace_root: "/test".into(),
                input: serde_json::json!({"block": blocked}),
            })
            .unwrap(),
        )
        .await
    }
}

fn init_params(settings: Value, callbacks: bool) -> Value {
    serde_json::to_value(InitContext {
        agena_version: "test".into(),
        workspace_root: "/test".into(),
        plugin_id: "test.instance".parse().unwrap(),
        settings,
        protocol_version: crate::rpc::PROTOCOL_VERSION,
        host_callback_url: callbacks.then(|| "http://127.0.0.1:1/callback".into()),
        host_callback_token: callbacks.then(|| "test-bearer".into()),
    })
    .unwrap()
}

fn init_headers(owner: &str, revision: &str) -> HeaderMap {
    let mut headers = owner_headers(Some(owner));
    headers.insert(EXPECTED_REVISION_HEADER, revision.parse().unwrap());
    headers
}

fn owner_headers(owner: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(owner) = owner {
        headers.insert(INSTANCE_HEADER, owner.parse().unwrap());
    }
    headers
}

fn success(response: Response) -> Value {
    match response.payload {
        ResponsePayload::Ok { result } => result,
        ResponsePayload::Err { error } => panic!("unexpected failure: {error:?}"),
    }
}

fn failure(response: Response) -> ErrorObject {
    match response.payload {
        ResponsePayload::Err { error } => error,
        ResponsePayload::Ok { .. } => panic!("unexpected success"),
    }
}

#[tokio::test]
async fn replaced_instance_rejects_old_calls_shutdown_rebind_and_retained_host_clients() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let old_host = fixture.observed.hosts.lock().unwrap()[0].clone();
    assert_eq!(old_host.read_config(None).await.unwrap()["host"], "working");
    let revision = fixture.revision().await.unwrap();
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
    for owner in [None, Some("first"), Some("unknown")] {
        failure(fixture.invoke(owner, false, false).await);
        failure(
            fixture
                .call(
                    owner_headers(owner),
                    method::META_SHUTDOWN,
                    serde_json::json!({}),
                )
                .await,
        );
        failure(
            fixture
                .call(
                    owner_headers(owner),
                    method::META_HOST_REBIND,
                    serde_json::json!({"expected": {}, "next": {}}),
                )
                .await,
        );
    }
    assert_eq!(
        old_host.read_config(None).await.unwrap_err().kind,
        PluginErrorKind::HostUnavailable
    );
    let new_host = fixture.observed.hosts.lock().unwrap()[1].clone();
    assert_eq!(new_host.read_config(None).await.unwrap()["host"], "working");
    success(fixture.invoke(Some("second"), false, false).await);
    assert_eq!(fixture.observed.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
    success(
        fixture
            .call(
                owner_headers(Some("second")),
                method::META_SHUTDOWN,
                serde_json::json!({}),
            )
            .await,
    );
    success(
        fixture
            .call(
                owner_headers(Some("second")),
                method::META_SHUTDOWN,
                serde_json::json!({}),
            )
            .await,
    );
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stale_revision_and_missing_revision_cannot_repeat_initialization() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    for (owner, revision) in [
        (Some("second"), Some("-")),
        (Some("second"), None),
        (None, None),
    ] {
        failure(
            fixture
                .init(owner, revision, serde_json::json!({}), false)
                .await,
        );
    }
    let revision = fixture.revision().await.unwrap();
    failure(
        fixture
            .init(Some("first"), Some(&revision), serde_json::json!({}), false)
            .await,
    );
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    success(fixture.invoke(Some("first"), false, false).await);
}

#[tokio::test]
async fn concurrent_initializers_with_the_same_revision_have_one_winner() {
    let fixture = Fixture::new();
    let (a, b) = tokio::join!(
        fixture.init(Some("first"), Some("-"), serde_json::json!({}), false),
        fixture.init(Some("second"), Some("-"), serde_json::json!({}), false),
    );
    assert_eq!(
        [a, b]
            .iter()
            .filter(|response| matches!(response.payload, ResponsePayload::Ok { .. }))
            .count(),
        1
    );
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn replacement_waits_for_cancelled_ordinary_plugin_work_to_drop() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    let mut invoke = Box::pin(fixture.invoke(Some("first"), true, false));
    tokio::select! {
        response = &mut invoke => panic!("call should block: {:?}", response.payload),
        _ = fixture.observed.invoke.entered.notified() => {}
    }
    let (replacement, cancelled) = tokio::join!(
        fixture.init(
            Some("second"),
            Some(&revision),
            serde_json::json!({}),
            false
        ),
        invoke
    );
    success(replacement);
    failure(cancelled);
    assert!(fixture.observed.invoke.dropped.load(Ordering::SeqCst));
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn replacement_waits_for_the_stream_plugin_future_to_drop() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), true)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    success(fixture.invoke(Some("first"), false, true).await);
    tokio::time::timeout(WAIT, fixture.observed.stream.entered.notified())
        .await
        .unwrap();
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert!(fixture.observed.stream.dropped.load(Ordering::SeqCst));
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.state.dispatch_slots.available_permits(), 64);
}

#[tokio::test]
async fn cancelling_initialization_stops_its_proxy_and_owns_cleanup() {
    let fixture = Fixture::new();
    let mut init = Box::pin(fixture.init(
        Some("first"),
        Some("-"),
        serde_json::json!({"block": true}),
        false,
    ));
    tokio::select! {
        response = &mut init => panic!("init should block: {:?}", response.payload),
        _ = fixture.observed.init.entered.notified() => {}
    }
    let host = fixture.observed.hosts.lock().unwrap()[0].clone();
    drop(init);
    assert!(fixture.observed.init.dropped.load(Ordering::SeqCst));
    assert_eq!(
        host.read_config(None).await.unwrap_err().kind,
        PluginErrorKind::HostUnavailable
    );
    tokio::time::timeout(WAIT, async {
        while fixture.observed.shutdowns.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let revision = fixture.revision().await.unwrap();
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_initialization_is_retired_and_cleaned_before_retry() {
    let fixture = Fixture::new();
    failure(
        fixture
            .init(
                Some("first"),
                Some("-"),
                serde_json::json!({"fail": true}),
                false,
            )
            .await,
    );
    failure(fixture.invoke(Some("first"), false, false).await);
    let revision = fixture.revision().await.unwrap();
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cleanup_failure_prevents_reinitialization_and_can_be_retried() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    fixture.observed.fail_shutdown.store(true, Ordering::SeqCst);
    failure(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    failure(fixture.invoke(Some("first"), false, false).await);
    fixture
        .observed
        .fail_shutdown
        .store(false, Ordering::SeqCst);
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 2);
}

async fn rejected_factory_preserves_the_live_instance(panics: bool) {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    let failure_mode = if panics {
        &fixture.observed.factory_panics
    } else {
        &fixture.observed.manifest_drifts
    };
    failure_mode.store(true, Ordering::SeqCst);
    failure(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.revision().await.as_deref(), Some(revision.as_str()));
    success(fixture.invoke(Some("first"), false, false).await);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    failure_mode.store(false, Ordering::SeqCst);
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    success(fixture.invoke(Some("second"), false, false).await);
}

#[tokio::test]
async fn factory_panic_preserves_the_live_instance() {
    rejected_factory_preserves_the_live_instance(true).await;
}

#[tokio::test]
async fn factory_manifest_change_preserves_the_live_instance() {
    rejected_factory_preserves_the_live_instance(false).await;
}

#[tokio::test]
async fn invalid_initialization_does_not_retire_the_live_instance() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    let valid = init_params(serde_json::json!({}), false);
    let mut malformed = vec![
        Value::Null,
        serde_json::json!([
            "test",
            "/test",
            "test.instance",
            null,
            null,
            {},
            crate::rpc::PROTOCOL_VERSION
        ]),
    ];
    for (field, value) in [
        ("protocol_version", serde_json::json!(999)),
        ("host_callback_url", serde_json::json!("file:///test")),
        ("host_callback_url", serde_json::json!("invalid-url")),
        ("host_callback_token", serde_json::json!("orphan-token")),
        ("config", serde_json::json!({"obsolete": true})),
    ] {
        let mut params = valid.clone();
        params[field] = value;
        malformed.push(params);
    }
    for token in ["", "invalid\r\ntoken"] {
        let mut params = init_params(serde_json::json!({}), true);
        params["host_callback_token"] = serde_json::json!(token);
        malformed.push(params);
    }
    for params in malformed {
        failure(
            fixture
                .call(init_headers("second", &revision), method::META_INIT, params)
                .await,
        );
        assert_eq!(fixture.revision().await.as_deref(), Some(revision.as_str()));
        success(fixture.invoke(Some("first"), false, false).await);
    }
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn malformed_instance_headers_cannot_initialize_or_act_on_a_live_instance() {
    use axum::http::HeaderValue;

    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    for name in [INSTANCE_HEADER, EXPECTED_REVISION_HEADER] {
        for value in [
            HeaderValue::from_static(""),
            HeaderValue::from_str(&"a".repeat(129)).unwrap(),
            HeaderValue::from_static("first,second"),
            HeaderValue::from_static("first second"),
            HeaderValue::from_bytes(&[0xff]).unwrap(),
        ] {
            let mut headers = init_headers("second", &revision);
            headers.insert(name, value);
            failure(
                fixture
                    .call(
                        headers,
                        method::META_INIT,
                        init_params(serde_json::json!({}), false),
                    )
                    .await,
            );
        }
        let mut headers = init_headers("second", &revision);
        let duplicate = headers[name].clone();
        headers.append(name, duplicate);
        failure(
            fixture
                .call(
                    headers,
                    method::META_INIT,
                    init_params(serde_json::json!({}), false),
                )
                .await,
        );
    }
    for method_name in [
        method::HOOK_TOOL_INVOKE,
        method::META_SHUTDOWN,
        method::META_HOST_REBIND,
    ] {
        let mut headers = owner_headers(Some("first"));
        headers.append(INSTANCE_HEADER, HeaderValue::from_static("first"));
        failure(
            fixture
                .call(headers, method_name, serde_json::json!({}))
                .await,
        );
    }
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    success(fixture.invoke(Some("first"), false, false).await);
}

async fn cancelled_retirement_owns_cleanup(method_name: &str) {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    fixture
        .observed
        .block_shutdown
        .store(true, Ordering::SeqCst);
    let (headers, params) = if method_name == method::META_INIT {
        (
            init_headers("second", &revision),
            init_params(serde_json::json!({}), false),
        )
    } else {
        (owner_headers(Some("first")), serde_json::json!({}))
    };
    let mut retirement = Box::pin(fixture.call(headers, method_name, params));
    tokio::select! {
        response = &mut retirement => panic!("shutdown should block: {:?}", response.payload),
        _ = fixture.observed.shutdown.entered.notified() => {}
    }
    drop(retirement);
    assert!(fixture.observed.shutdown.dropped.load(Ordering::SeqCst));
    failure(fixture.invoke(Some("first"), false, false).await);
    tokio::time::timeout(WAIT, fixture.observed.shutdown.entered.notified())
        .await
        .unwrap();
    fixture
        .observed
        .block_shutdown
        .store(false, Ordering::SeqCst);
    fixture.observed.shutdown.release.notify_one();
    success(
        fixture
            .init(Some("third"), Some(&revision), serde_json::json!({}), false)
            .await,
    );
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 2);
    success(fixture.invoke(Some("third"), false, false).await);
}

#[tokio::test]
async fn cancelled_shutdown_owns_cleanup() {
    cancelled_retirement_owns_cleanup(method::META_SHUTDOWN).await;
}

#[tokio::test]
async fn cancelled_replacement_owns_predecessor_cleanup() {
    cancelled_retirement_owns_cleanup(method::META_INIT).await;
}

#[tokio::test(start_paused = true)]
async fn initialization_deadline_cancels_plugin_code_and_allows_a_successor() {
    let fixture = Fixture::new();
    let started = tokio::time::Instant::now();
    let error = control(
        fixture.state.clone(),
        &init_headers("first", "-"),
        method::META_INIT,
        init_params(serde_json::json!({"block": true}), false),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::Timeout);
    assert_eq!(started.elapsed(), CONTROL_TIMEOUT);
    assert!(fixture.observed.init.dropped.load(Ordering::SeqCst));
    failure(fixture.invoke(Some("first"), false, false).await);
    let revision = fixture.revision().await.unwrap();
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 1);
    success(fixture.invoke(Some("second"), false, false).await);
}

#[tokio::test(start_paused = true)]
async fn shutdown_and_its_cleanup_retry_have_bounded_deadlines() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    fixture
        .observed
        .block_shutdown
        .store(true, Ordering::SeqCst);
    let started = tokio::time::Instant::now();
    let error = control(
        fixture.state.clone(),
        &owner_headers(Some("first")),
        method::META_SHUTDOWN,
        serde_json::json!({}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::Timeout);
    assert_eq!(started.elapsed(), CONTROL_TIMEOUT);
    assert!(fixture.observed.shutdown.dropped.load(Ordering::SeqCst));
    // Let the owned cleanup retry acquire control, then wait for it to release
    // control at its own deadline. No client disconnect is needed for either.
    tokio::task::yield_now().await;
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 2);
    drop(fixture.state.instances.control.lock().await);
    assert_eq!(started.elapsed(), CONTROL_TIMEOUT * 2);
    failure(fixture.invoke(Some("first"), false, false).await);
    fixture
        .observed
        .block_shutdown
        .store(false, Ordering::SeqCst);
    let revision = fixture.revision().await.unwrap();
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 3);
    success(fixture.invoke(Some("second"), false, false).await);
}

#[tokio::test(start_paused = true)]
async fn lifecycle_lock_wait_has_a_deadline_without_stopping_its_owner() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    let lock = fixture.state.instances.control.lock().await;
    let error = control(
        fixture.state.clone(),
        &init_headers("second", &revision),
        method::META_INIT,
        init_params(serde_json::json!({}), false),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::Timeout);
    assert_eq!(fixture.observed.inits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    success(fixture.invoke(Some("first"), false, false).await);
    drop(lock);
}

#[tokio::test(start_paused = true)]
async fn rebind_deadline_preserves_the_live_callback_binding() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), false)
            .await,
    );
    let host = fixture.observed.hosts.lock().unwrap()[0].clone();
    let instance = fixture.state.instances.current().unwrap();
    let callback_lock = instance.callback_client.write().await;
    let error = control(
        fixture.state.clone(),
        &owner_headers(Some("first")),
        method::META_HOST_REBIND,
        serde_json::json!({
            "expected": {},
            "next": {"url": "http://127.0.0.1:1/callback", "token": "next"},
        }),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::Timeout);
    assert!(callback_lock.is_none());
    assert_eq!(host.read_config(None).await.unwrap()["host"], "working");
    assert_eq!(fixture.observed.shutdowns.load(Ordering::SeqCst), 0);
    drop(callback_lock);
    success(fixture.invoke(Some("first"), false, false).await);
}

#[tokio::test]
async fn replacement_cancels_all_stream_tasks_when_dispatch_capacity_is_full() {
    let fixture = Fixture::new();
    success(
        fixture
            .init(Some("first"), Some("-"), serde_json::json!({}), true)
            .await,
    );
    let revision = fixture.revision().await.unwrap();
    for _ in 0..64 {
        success(fixture.invoke(Some("first"), false, true).await);
    }
    tokio::time::timeout(WAIT, async {
        while fixture.observed.streams_started.load(Ordering::SeqCst) != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.state.dispatch_slots.available_permits(), 0);
    failure(fixture.invoke(Some("first"), false, false).await);
    success(
        fixture
            .init(
                Some("second"),
                Some(&revision),
                serde_json::json!({}),
                false,
            )
            .await,
    );
    assert_eq!(fixture.observed.streams_finished.load(Ordering::SeqCst), 64);
    assert_eq!(fixture.state.dispatch_slots.available_permits(), 64);
    success(fixture.invoke(Some("second"), false, false).await);
}
