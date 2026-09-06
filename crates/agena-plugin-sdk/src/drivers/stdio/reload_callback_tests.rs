use super::*;
use crate::host_api::{
    HostCallbackContext, HostConfigReloadState, run_in_isolated_host_callback_context,
};

#[tokio::test]
async fn reload_request_and_status_stdio_callbacks_preserve_authority_and_distinct_results() {
    let (tx, mut rx) = mpsc::channel(2);
    let pending = Arc::new(StdMutex::new(HashMap::new()));
    let client = StdioHostClient {
        tx,
        pending: pending.clone(),
        next_id: Arc::new(AtomicI64::new(1)),
    };
    let context = HostCallbackContext {
        authority_token: Some("test-authority".into()),
        plugin_id: Some("test.reload".into()),
        session_id: Some(11),
        call_id: Some(22),
        ..Default::default()
    };
    let callback = tokio::spawn(run_in_isolated_host_callback_context(
        context.clone(),
        async move {
            let accepted = client.request_config_reload().await.unwrap();
            assert!(accepted.started);
            client
                .config_reload_status(HostConfigReloadStatusRequest {
                    task_id: accepted.task_id,
                })
                .await
                .unwrap()
        },
    ));
    for (method, result) in [
        (
            method::HOST_CONFIG_RELOAD_REQUEST,
            serde_json::json!({"task_id":"task-1","started":true}),
        ),
        (
            method::HOST_CONFIG_RELOAD_STATUS,
            serde_json::json!({"task_id":"task-1","state":{"status":"succeeded"}}),
        ),
    ] {
        let bytes = rx.recv().await.unwrap();
        let request: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request.method, method);
        let params = request.params.unwrap();
        assert_eq!(params["context"], serde_json::to_value(&context).unwrap());
        if method == method::HOST_CONFIG_RELOAD_STATUS {
            assert_eq!(params["request"]["task_id"], "task-1");
        }
        pending
            .lock()
            .unwrap()
            .remove(&request.id)
            .unwrap()
            .send(Response {
                jsonrpc: JsonRpcVersion,
                id: request.id,
                payload: ResponsePayload::Ok { result },
            })
            .unwrap();
    }
    let status = callback.await.unwrap();
    assert_eq!(status.task_id, "task-1");
    assert!(matches!(status.state, HostConfigReloadState::Succeeded {}));
    assert!(pending.lock().unwrap().is_empty());
}

#[test]
fn reload_acceptance_and_status_reject_legacy_or_incompatible_shapes() {
    assert!(
        serde_json::from_value::<HostConfigReloadRequestResponse>(
            serde_json::json!({"previous_generation":1,"generation":2,"loaded_at":"now"})
        )
        .is_err()
    );
    for state in [
        serde_json::json!({"status":"running","config":{}}),
        serde_json::json!({"status":"succeeded","generation":2}),
        serde_json::json!({"status":"cancelled","generation":2}),
        serde_json::json!({"status":"failed"}),
        serde_json::json!({"status":"queued"}),
    ] {
        assert!(
            serde_json::from_value::<HostConfigReloadState>(state.clone()).is_err(),
            "accepted incompatible state: {state}"
        );
    }
    assert!(
        serde_json::from_value::<HostConfigReloadStatusRequest>(
            serde_json::json!({"task_id":"task-1","config":{}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<HostConfigReloadStatusResponse>(
            serde_json::json!({"task_id":"task-1","state":{"status":"running"},"generation":2})
        )
        .is_err()
    );
}

#[tokio::test]
async fn reload_stdio_callback_preserves_host_failure_identity_and_retry_semantics() {
    let (tx, mut rx) = mpsc::channel(1);
    let pending = Arc::new(StdMutex::new(HashMap::new()));
    let client = StdioHostClient {
        tx,
        pending: pending.clone(),
        next_id: Arc::new(AtomicI64::new(1)),
    };
    let callback = tokio::spawn(async move { client.request_config_reload().await });
    let request: Request = serde_json::from_slice(&rx.recv().await.unwrap()).unwrap();
    let expected = PluginError::from_kind_with_public_detail(
        PluginErrorKind::HostUnavailable,
        "capacity 64",
        "Wait for a background task to finish and retry.",
    );
    pending
        .lock()
        .unwrap()
        .remove(&request.id)
        .unwrap()
        .send(Response {
            jsonrpc: JsonRpcVersion,
            id: request.id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: expected.to_string(),
                    data: expected.rpc_error_data(),
                },
            },
        })
        .unwrap();
    assert_eq!(callback.await.unwrap().unwrap_err(), expected);
    assert!(pending.lock().unwrap().is_empty());
}
