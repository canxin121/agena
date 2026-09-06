use super::*;
use serde_json::{Value, json};

const KEY: &str = "test.stream-authority";

struct RecordingTransport(Arc<Mutex<Vec<HostCallbackContext>>>);

#[async_trait::async_trait]
impl PluginTransport for RecordingTransport {
    async fn dispatch(&self, _: &str, _: Value) -> Result<Value, crate::TransportError> {
        unreachable!("stream ingress must use its own transport entry");
    }

    async fn ingest_stream_event(&self, _: &str, _: Value) -> Result<bool, crate::TransportError> {
        self.0
            .lock()
            .unwrap()
            .push(host_api::current_host_callback_context().unwrap());
        Ok(true)
    }
}

async fn fixture() -> (Arc<HostHandle>, Arc<Mutex<Vec<HostCallbackContext>>>) {
    let host = Arc::new(HostHandle::new(Arc::new(
        crate::sdk::host_api::NoopHostClient,
    )));
    host.begin_plugin_instance(KEY.parse().unwrap());
    let received = Arc::new(Mutex::new(Vec::new()));
    host.register_plugin_transport(
        KEY.parse().unwrap(),
        Arc::new(RecordingTransport(received.clone())),
    )
    .await
    .unwrap();
    (host, received)
}

#[tokio::test]
async fn untrusted_stream_authority_is_rejected_before_transport_dispatch() {
    let (host, received) = fixture().await;
    let (expired, lease) = host.issue_callback_authority(
        &KEY.parse().unwrap(),
        HostCallbackContext {
            call_id: Some(1),
            ..Default::default()
        },
    );
    drop(lease);
    let other = "test.other".parse().unwrap();
    host.begin_plugin_instance(other);
    let (foreign, _foreign_lease) = host.issue_callback_authority(
        &"test.other".parse().unwrap(),
        HostCallbackContext::default(),
    );
    let contexts = [
        json!({}),
        json!({"call_id": 1}),
        json!({"authority_token": "not-issued"}),
        serde_json::to_value(expired).unwrap(),
        serde_json::to_value(foreign).unwrap(),
    ];
    for method in [
        method::TOOL_STREAM_CHUNK,
        method::TOOL_STREAM_END,
        method::TOOL_STREAM_ERROR,
    ] {
        for context in &contexts {
            let error = host
                .ingest_stream_event_for_plugin(KEY, method, json!({"context": context}))
                .await
                .unwrap_err();
            assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
        }
        let error = host
            .ingest_stream_event_for_plugin(KEY, method, json!({"context": []}))
            .await
            .unwrap_err();
        assert_eq!(error.kind, PluginErrorKind::InvalidParams);
    }
    assert!(
        received.lock().unwrap().is_empty(),
        "untrusted events reached the transport"
    );
}

#[tokio::test]
async fn stream_transport_observes_only_the_validated_request_context() {
    let (host, received) = fixture().await;
    let (context, _authority) = host.issue_callback_authority(
        &KEY.parse().unwrap(),
        HostCallbackContext {
            call_id: Some(1),
            ..Default::default()
        },
    );
    let outer = HostCallbackContext {
        session_id: Some(999),
        workspace_root: Some("/outer".into()),
        tool_name: Some("outer".into()),
        ..Default::default()
    };
    host_api::run_in_host_callback_context(outer.clone(), async {
        assert!(
            host.ingest_stream_event_for_plugin(
                KEY,
                method::TOOL_STREAM_CHUNK,
                json!({"context": context})
            )
            .await
            .unwrap()
        );
        assert_eq!(host_api::current_host_callback_context(), Some(outer));
    })
    .await;
    assert_eq!(*received.lock().unwrap(), [context]);
}
