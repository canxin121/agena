use super::*;
use crate::sdk::rpc::{JsonRpcVersion, Request, RequestId, ResponsePayload};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
struct Probe {
    started: mpsc::UnboundedSender<String>,
    stopped: mpsc::UnboundedSender<String>,
}

struct RunningBody {
    name: String,
    stopped: mpsc::UnboundedSender<String>,
}

impl Drop for RunningBody {
    fn drop(&mut self) {
        let _ = self.stopped.send(self.name.clone());
    }
}

impl Probe {
    async fn wait<T>(&self, name: String) -> crate::sdk::Result<T> {
        let _body = RunningBody {
            name: name.clone(),
            stopped: self.stopped.clone(),
        };
        self.started.send(name).unwrap();
        std::future::pending().await
    }
}

struct ProbeHost(Probe);

#[async_trait::async_trait]
impl HostClient for ProbeHost {
    async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> crate::sdk::Result<()> {
        unreachable!()
    }
    async fn subscribe_events(&self, _: EventFilter) -> crate::sdk::Result<EventSubscription> {
        unreachable!()
    }
    async fn read_config(&self, _: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        unreachable!()
    }
    async fn invoke_tool(
        &self,
        name: String,
        _: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.0.wait(name).await
    }
}

struct ProbeService(Probe);

#[async_trait::async_trait]
impl crate::sdk::Plugin for ProbeService {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test", "service", "1.0.0")
    }
    async fn service_invoke(
        &self,
        _: PluginServiceInvokeInput,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.0.wait("service".into()).await
    }
}

fn probe() -> (
    Probe,
    mpsc::UnboundedReceiver<String>,
    mpsc::UnboundedReceiver<String>,
) {
    let (started, starts) = mpsc::unbounded_channel();
    let (stopped, stops) = mpsc::unbounded_channel();
    (Probe { started, stopped }, starts, stops)
}

async fn receive(receiver: &mut mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

fn handle(probe: Probe) -> (Arc<HostHandle>, PluginKey) {
    let mut handle = HostHandle::new(Arc::new(ProbeHost(probe)));
    handle.callback_base_url = Some("http://127.0.0.1/callbacks".into());
    let plugin: PluginKey = "test.cancellation".parse().unwrap();
    handle.begin_plugin_instance(plugin.clone());
    (Arc::new(handle), plugin)
}

async fn origin(
    handle: Arc<HostHandle>,
    plugin: PluginKey,
    call_id: i64,
) -> (
    HostCallbackContext,
    tokio::task::JoinHandle<()>,
    oneshot::Sender<()>,
) {
    let (context_tx, context_rx) = oneshot::channel();
    let (finish, finished) = oneshot::channel();
    let task = tokio::spawn(async move {
        let context = HostCallbackContext {
            session_id: Some(1),
            call_id: Some(call_id),
            workspace_root: Some("/workspace".into()),
            ..Default::default()
        };
        handle
            .run_in_authorized_callback_context(&plugin, context, async {
                context_tx
                    .send(host_api::current_host_callback_context().unwrap())
                    .unwrap();
                let _ = finished.await;
            })
            .await;
    });
    (context_rx.await.unwrap(), task, finish)
}

async fn invoke_rpc(
    handle: Arc<HostHandle>,
    plugin: PluginKey,
    context: HostCallbackContext,
    name: &str,
) -> crate::sdk::Result<serde_json::Value> {
    handle
        .handle_call_for_plugin(
            &plugin.to_string(),
            method::HOST_TOOL_INVOKE,
            json!({"tool": name, "input": {}, "context": context}),
        )
        .await
}

#[tokio::test]
async fn dropping_origin_stops_admitted_rpc_body_without_cancelling_a_peer() {
    let (probe, mut starts, mut stops) = probe();
    let (handle, plugin) = handle(probe);
    let (first_context, first, _first_finish) = origin(handle.clone(), plugin.clone(), 1).await;
    let (second_context, second, second_finish) = origin(handle.clone(), plugin.clone(), 2).await;
    let first_callback = tokio::spawn(invoke_rpc(
        handle.clone(),
        plugin.clone(),
        first_context,
        "first",
    ));
    assert_eq!(receive(&mut starts).await, "first");
    let second_callback = tokio::spawn(invoke_rpc(
        handle.clone(),
        plugin.clone(),
        second_context,
        "second",
    ));
    assert_eq!(receive(&mut starts).await, "second");
    first.abort();
    let _ = first.await;
    assert_eq!(receive(&mut stops).await, "first");
    let error = first_callback.await.unwrap().unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::Cancelled);
    assert!(!second_callback.is_finished());
    second_finish.send(()).unwrap();
    second.await.unwrap();
    assert_eq!(receive(&mut stops).await, "second");
    assert_eq!(
        second_callback.await.unwrap().unwrap_err().kind,
        PluginErrorKind::Cancelled
    );
}

#[tokio::test]
async fn natural_origin_completion_stops_admitted_inproc_callback_body() {
    let (probe, mut starts, mut stops) = probe();
    let (handle, plugin) = handle(probe);
    let (context, origin, finish) = origin(handle.clone(), plugin.clone(), 3).await;
    let scoped = handle.scoped_host_client(plugin.to_string());
    let callback = tokio::spawn(host_api::run_in_isolated_host_callback_context(
        context,
        async move { scoped.invoke_tool("inproc".into(), json!({})).await },
    ));
    assert_eq!(receive(&mut starts).await, "inproc");
    finish.send(()).unwrap();
    origin.await.unwrap();
    assert_eq!(receive(&mut stops).await, "inproc");
    assert_eq!(
        callback.await.unwrap().unwrap_err().kind,
        PluginErrorKind::Cancelled
    );
}

#[tokio::test]
async fn dropping_origin_stops_admitted_http_callback_and_nested_service() {
    let (probe, mut starts, mut stops) = probe();
    let (handle, plugin) = handle(probe.clone());
    let provider: PluginKey = "test.service".parse().unwrap();
    handle.begin_plugin_instance(provider.clone());
    handle
        .register_plugin_transport(
            provider.clone(),
            Arc::new(InProcessTransport::new(ProbeService(probe))),
        )
        .await
        .unwrap();
    let binding = PluginServiceBinding {
        consumer: plugin.to_string(),
        provider: provider.to_string(),
        service: "test_service".into(),
        api_version: 1,
        optional: false,
        methods: BTreeMap::from([(
            "wait".into(),
            crate::sdk::PluginServiceMethod::bounded_json("wait", 4096, 16),
        )]),
    };
    handle
        .install_service_bindings(BTreeMap::from([(
            PluginServiceBindingKey {
                consumer: plugin.to_string(),
                service: binding.service.clone(),
                api_version: 1,
            },
            binding,
        )]))
        .await;
    let callback_token = handle.callback_token(&plugin.to_string()).await.unwrap();
    let (context, origin, _finish) = origin(handle.clone(), plugin.clone(), 4).await;
    let dispatcher = handle.callback_dispatcher();
    let callback = tokio::spawn(async move {
        dispatcher.dispatch(&plugin.to_string(), Some(&callback_token), Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(5),
            method: method::HOST_SERVICE_INVOKE.into(),
            params: Some(json!({"context": context, "request": PluginServiceInvokeInput {
                service: "test_service".into(), api_version: 1, method: "wait".into(), input: json!({}),
            }})),
            context: None,
        }).await.unwrap()
    });
    assert_eq!(receive(&mut starts).await, "service");
    origin.abort();
    let _ = origin.await;
    assert_eq!(receive(&mut stops).await, "service");
    let ResponsePayload::Err { error } = callback.await.unwrap().payload else {
        panic!("an interrupted callback must return cancellation");
    };
    assert_eq!(
        PluginError::from_rpc_error(error, "test callback").kind,
        PluginErrorKind::Cancelled
    );
}

#[tokio::test]
async fn revocation_between_context_validation_and_callback_admission_fails_closed() {
    let (probe, mut starts, _stops) = probe();
    let (handle, plugin) = handle(probe);
    let (context, lease) = handle.issue_callback_authority(&plugin, HostCallbackContext::default());
    let trusted = handle
        .validated_callback_context(Some(plugin.to_string()), Some(context))
        .unwrap();
    drop(lease);
    let result = handle
        .run_in_callback_completion(&trusted, async {
            panic!("expired callback body must not start");
            #[allow(unreachable_code)]
            Ok(())
        })
        .await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::Cancelled);
    assert!(starts.try_recv().is_err());
}
