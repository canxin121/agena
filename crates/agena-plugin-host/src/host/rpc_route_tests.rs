//! Routes the SDK's driver declarations name must reach a dispatch arm here.
//!
//! A constant with no arm still compiles and ships in the SDK; the mismatch
//! surfaces only at runtime, as `NotImplemented` on a live transport. These
//! tests hold the three routes that had drifted that way, and keep one
//! negative case so the fallback arm stays observable.

use super::*;
use serde_json::json;
use std::sync::Mutex;

#[derive(Default)]
struct Observed {
    session_streams: Mutex<Vec<(Option<i64>, String)>>,
}
struct RouteHost {
    observed: Arc<Observed>,
}

#[async_trait::async_trait]
impl HostClient for RouteHost {
    async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> crate::sdk::Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> crate::sdk::Result<EventSubscription> {
        Err(PluginError::from_kind(
            PluginErrorKind::HostUnavailable,
            "no event bus in this fixture",
        ))
    }
    async fn read_config(&self, _: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn invoke_tool(
        &self,
        _: String,
        _: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        unreachable!()
    }
    async fn session_stream(
        &self,
        request: HostSessionStreamRequest,
    ) -> crate::sdk::Result<EventSubscription> {
        self.observed
            .session_streams
            .lock()
            .unwrap()
            .push((request.session_id, format!("{:?}", request.kind)));
        Ok(EventSubscription {
            id: "stream-1".into(),
        })
    }
}

fn fixture() -> (Arc<HostHandle>, PluginKey, Arc<Observed>) {
    let observed = Arc::new(Observed::default());
    let host = RouteHost {
        observed: Arc::clone(&observed),
    };
    let key: PluginKey = "test.routes".parse().unwrap();
    let handle = HostHandle::new(Arc::new(host));
    handle.begin_plugin_instance(key.clone());
    (Arc::new(handle), key, observed)
}

async fn call(
    handle: &HostHandle,
    plugin: &PluginKey,
    method: &str,
    params: serde_json::Value,
) -> crate::sdk::Result<serde_json::Value> {
    handle
        .handle_call_for_plugin(&plugin.to_string(), method, params)
        .await
}

fn contribution(id: &str) -> serde_json::Value {
    json!({"request": {"contribution": {
        "id": id,
        "kind": "status_line_text",
        "priority": 5,
        "content": {"kind": "text", "text": "busy"},
    }}})
}

#[tokio::test]
async fn session_stream_route_forwards_the_request_to_the_host_client() {
    let (handle, plugin, observed) = fixture();
    let response = call(
        &handle,
        &plugin,
        method::HOST_SESSION_STREAM,
        json!({"request": {"session_id": 7, "kind": "parts"}}),
    )
    .await
    .unwrap();
    assert_eq!(response, json!({"id": "stream-1"}));
    assert_eq!(
        *observed.session_streams.lock().unwrap(),
        vec![(Some(7), "Parts".to_string())]
    );
}

#[tokio::test]
async fn hook_list_route_returns_the_host_hook_catalog() {
    let (handle, plugin, _) = fixture();
    handle
        .set_plugin_hook_catalog(HostHookRegistration {
            plugin_id: plugin.clone(),
            trust_level: "trusted".into(),
            trust_status: "verified".into(),
            provenance: vec![],
            source: "manifest".into(),
            source_path: None,
            manifest_hash: None,
            hooks: vec![HostHookDescriptor {
                name: "tool.before".into(),
                handler_id: "guard".into(),
                summary: "Guards tool calls".into(),
                help: "Runs before every tool invocation.".into(),
                translations: BTreeMap::new(),
                priority: 0,
                filters: Default::default(),
                is_async: false,
                enabled: true,
                trust_level: "trusted".into(),
                trust_status: "verified".into(),
                source: "manifest".into(),
                source_path: None,
                current_hash: None,
            }],
        })
        .unwrap();
    let response = call(&handle, &plugin, method::HOST_HOOK_LIST, json!({}))
        .await
        .unwrap();
    assert_eq!(response["hooks"][0]["plugin_id"], json!(plugin.to_string()));
    assert_eq!(response["hooks"][0]["hooks"][0]["handler_id"], "guard");
}

#[tokio::test]
async fn display_contribute_route_registers_a_contribution_for_the_calling_plugin() {
    let (handle, plugin, _) = fixture();
    call(
        &handle,
        &plugin,
        method::HOST_UI_DISPLAY_CONTRIBUTE,
        contribution("state"),
    )
    .await
    .unwrap();
    let contributions = handle.display_list_response();
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0].plugin_id, plugin);
    assert_eq!(contributions[0].contribution.id, "state");
}

#[tokio::test]
async fn display_contribute_route_refuses_a_callback_with_no_plugin_identity() {
    let (handle, _, _) = fixture();
    // Attribution is the host's, so an unattributed callback has no owner to
    // register the contribution under.
    let error = handle
        .handle_call(method::HOST_UI_DISPLAY_CONTRIBUTE, contribution("state"))
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::HostUnavailable);
    assert!(handle.display_list_response().is_empty());
}

#[tokio::test]
async fn a_route_with_no_dispatch_arm_is_still_reported_as_not_implemented() {
    let (handle, plugin, _) = fixture();
    let error = call(
        &handle,
        &plugin,
        "host/route.that.does.not.exist",
        json!({}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::NotImplemented);
}
