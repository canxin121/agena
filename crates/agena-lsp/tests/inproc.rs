//! In-memory smoke test for LspClient: stand up a fake server, run an
//! initialize → definition → publishDiagnostics round trip, verify the
//! typed cache picks them up.

use std::sync::Arc;

use agena_lsp::client::LspClient;
use agena_lsp::protocol::{
    InboundMessage, JSONRPC_VERSION, JsonRpcNotification, JsonRpcResponse, RequestId,
};
use agena_lsp::transport::InMemoryTransport;
use lsp_types::{Diagnostic, Location, Position, PublishDiagnosticsParams, Range, Uri};
use serde_json::{Value, json};
use tokio::sync::mpsc;

fn spawn_fake_server(
    mut from_client: mpsc::UnboundedReceiver<Value>,
    to_client: mpsc::UnboundedSender<InboundMessage>,
) {
    tokio::spawn(async move {
        while let Some(msg) = from_client.recv().await {
            let id = msg.get("id").cloned();
            let method = msg
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or_default()
                .to_string();
            let Some(id) = id else {
                // notification — just drop
                continue;
            };
            let id: RequestId = serde_json::from_value(id).unwrap();
            let result = match method.as_str() {
                "initialize" => json!({
                    "capabilities": {},
                    "serverInfo": { "name": "fake", "version": "0.0.1" }
                }),
                "textDocument/definition" => json!({
                    "uri": "file:///tmp/lib.rs",
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": 0, "character": 5 }
                    }
                }),
                "shutdown" => Value::Null,
                _ => Value::Null,
            };
            let resp = JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id,
                result: Some(result),
                error: None,
            };
            let _ = to_client.send(InboundMessage::Response(resp));
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_definition_and_diagnostics_roundtrip() {
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (in_tx, in_rx) = mpsc::unbounded_channel();
    spawn_fake_server(out_rx, in_tx.clone());

    let transport: Arc<dyn agena_lsp::transport::LspTransport> =
        Arc::new(InMemoryTransport::new(out_tx, in_rx));
    let client = LspClient::new(transport);

    let init = client
        .initialize(None, "agena-test", "0.0.1", None)
        .await
        .unwrap();
    assert_eq!(init.server_info.unwrap().name, "fake");

    let uri: Uri = "file:///tmp/lib.rs".parse().unwrap();
    let response = client
        .definition(uri.clone(), Position::new(0, 0))
        .await
        .unwrap();
    let location: Location = match response.unwrap() {
        lsp_types::GotoDefinitionResponse::Scalar(l) => l,
        other => panic!("expected scalar Location, got {other:?}"),
    };
    assert_eq!(location.uri.to_string(), "file:///tmp/lib.rs");

    // Push a diagnostics notification and verify the typed cache picks it up.
    let diag = Diagnostic {
        range: Range::new(Position::new(0, 0), Position::new(0, 5)),
        message: "unused".to_string(),
        ..Default::default()
    };
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: vec![diag],
        version: None,
    };
    in_tx
        .send(InboundMessage::Notification(JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: "textDocument/publishDiagnostics".to_string(),
            params: Some(serde_json::to_value(params).unwrap()),
        }))
        .unwrap();

    // Give the reader loop a moment.
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        if !client.diagnostics_for(&uri).is_empty() {
            break;
        }
    }
    let got = client.diagnostics_for(&uri);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].message, "unused");
}
