use super::*;
use serde_json::json;

struct TestTransport {
    inbound: Mutex<mpsc::UnboundedReceiver<LspResult<InboundMessage>>>,
    outbound: mpsc::UnboundedSender<Value>,
}

#[async_trait::async_trait]
impl LspTransport for TestTransport {
    async fn send(&self, payload: Value) -> LspResult<()> {
        self.outbound
            .send(payload)
            .map_err(|error| LspError::transport_error(&error))
    }

    async fn recv(&self) -> LspResult<InboundMessage> {
        self.inbound
            .lock()
            .await
            .recv()
            .await
            .unwrap_or_else(|| Err(LspError::TransportClosed("test peer closed".into())))
    }

    async fn close(&self) -> LspResult<()> {
        Ok(())
    }
}

struct Peer {
    client: Arc<LspClient>,
    inbound: mpsc::UnboundedSender<LspResult<InboundMessage>>,
    outbound: mpsc::UnboundedReceiver<Value>,
}

impl Peer {
    fn new() -> Self {
        let (inbound, rx) = mpsc::unbounded_channel();
        let (tx, outbound) = mpsc::unbounded_channel();
        let client = LspClient::new(Arc::new(TestTransport {
            inbound: Mutex::new(rx),
            outbound: tx,
        }));
        Self {
            client,
            inbound,
            outbound,
        }
    }

    fn send(&self, message: Value) {
        self.inbound
            .send(Ok(InboundMessage::from_value(message).unwrap()))
            .unwrap();
    }

    async fn next(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(2), self.outbound.recv())
            .await
            .expect("client did not send an LSP message")
            .unwrap()
    }

    async fn initialize(&mut self, capabilities: Value) -> LspResult<InitializeResult> {
        let client = Arc::clone(&self.client);
        let initialization = tokio::spawn(async move {
            client
                .initialize_with_configuration(
                    Some("file:///project".parse().unwrap()),
                    "test",
                    "1",
                    None,
                    json!({"language": {"check": true}}),
                )
                .await
        });
        let request = self.next().await;
        assert_eq!(request["method"], "initialize");
        assert_eq!(
            request["params"]["workspaceFolders"][0]["uri"],
            "file:///project"
        );
        assert_eq!(
            request["params"]["capabilities"]["textDocument"]["publishDiagnostics"]["versionSupport"],
            true
        );
        assert_eq!(
            request["params"]["capabilities"]["general"]["positionEncodings"],
            json!(["utf-16"])
        );
        self.send(json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {"capabilities": capabilities}
        }));
        let result = initialization.await.unwrap();
        if result.is_ok() {
            assert_eq!(self.next().await["method"], "initialized");
        }
        result
    }

    fn publish(&self, uri: &Uri, version: Option<i32>, message: &str) {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "version": version,
                "diagnostics": [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                    "message": message,
                }],
            },
        }));
    }
}

#[tokio::test]
async fn answers_server_configuration_with_string_id() {
    let mut peer = Peer::new();
    peer.initialize(json!({"textDocumentSync": 1}))
        .await
        .unwrap();
    peer.send(json!({
        "jsonrpc": "2.0",
        "id": "server-configuration",
        "method": "workspace/configuration",
        "params": {"items": [{"section": "language.check"}, {"section": "unknown"}, {}]},
    }));
    let reply = peer.next().await;
    assert_eq!(reply["id"], "server-configuration");
    assert_eq!(
        reply["result"],
        json!([true, null, {"language": {"check": true}}])
    );
}

#[tokio::test]
async fn supports_incremental_replacement_and_rejects_other_position_encodings() {
    let mut peer = Peer::new();
    assert!(
        peer.initialize(json!({"positionEncoding": "utf-8"}))
            .await
            .is_err()
    );

    let mut peer = Peer::new();
    peer.initialize(json!({"textDocumentSync": {"openClose": true, "change": 2}}))
        .await
        .unwrap();
    let uri: Uri = "file:///project/example.rs".parse().unwrap();
    assert!(valid_position("a\r\n😀\r", Position::new(1, 2)));
    assert!(!valid_position("a\r\n😀\r", Position::new(1, 1)));
    assert!(valid_position("a\r\n😀\r", Position::new(2, 0)));
    assert!(!valid_position("a\r\n😀\r", Position::new(3, 0)));
    peer.client
        .sync_document(uri.clone(), "a\r\n😀z".into(), "rust")
        .await
        .unwrap();
    assert_eq!(peer.next().await["method"], "textDocument/didOpen");
    peer.client
        .sync_document(uri, "changed".into(), "rust")
        .await
        .unwrap();
    let change = peer.next().await;
    assert_eq!(change["method"], "textDocument/didChange");
    assert_eq!(
        change["params"]["contentChanges"][0],
        json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 1, "character": 3}},
            "text": "changed"
        })
    );
}

#[tokio::test]
async fn rejects_old_unversioned_diagnostics_and_invalidates_on_eof() {
    let mut peer = Peer::new();
    peer.initialize(json!({"textDocumentSync": 1}))
        .await
        .unwrap();
    let uri: Uri = "file:///project/file.rs".parse().unwrap();
    let before = peer
        .client
        .sync_document_with_receipt(uri.clone(), "old".into(), "rust")
        .await
        .unwrap();
    peer.next().await;
    peer.publish(&uri, None, "old error");
    let report = peer
        .client
        .wait_diagnostics(&uri, before.version, Duration::from_secs(2))
        .await;
    assert_eq!(report.state, "unversioned");
    assert_eq!(report.diagnostics[0].message, "old error");
    let after = peer
        .client
        .sync_document_with_receipt(uri.clone(), "new".into(), "rust")
        .await
        .unwrap();
    peer.next().await;
    let stale = peer.client.diagnostic_report(&uri, after.version);
    assert_eq!(stale.state, "stale");
    assert!(stale.diagnostics.is_empty());
    peer.publish(&uri, Some(after.version), "new error");
    let report = peer
        .client
        .wait_diagnostics(&uri, after.version, Duration::from_secs(2))
        .await;
    assert_eq!(report.state, "current");
    assert_eq!(report.diagnostics[0].message, "new error");

    let mut closed = peer.client.inner.closed.subscribe();
    peer.inbound
        .send(Err(LspError::TransportClosed("peer exited".into())))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), closed.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(!peer.client.is_alive());
    assert_eq!(
        peer.client.diagnostic_report(&uri, after.version).state,
        "disconnected"
    );
    assert!(
        peer.client
            .sync_document(uri, "new".into(), "rust")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancels_pending_request_and_serializes_document_queries() {
    let mut peer = Peer::new();
    peer.initialize(json!({"textDocumentSync": 1, "hoverProvider": true}))
        .await
        .unwrap();
    let uri: Uri = "file:///project/file.rs".parse().unwrap();
    let first_uri = uri.clone();
    let client = Arc::clone(&peer.client);
    let query = tokio::spawn(async move {
        let request_client = Arc::clone(&client);
        client
            .with_document(first_uri.clone(), "old".into(), "rust", |_| async move {
                request_client.hover(first_uri, Position::default()).await
            })
            .await
    });
    assert_eq!(peer.next().await["method"], "textDocument/didOpen");
    let hover = peer.next().await;
    assert_eq!(hover["method"], "textDocument/hover");

    let client = Arc::clone(&peer.client);
    let queued = tokio::spawn(async move { client.sync_document(uri, "new".into(), "rust").await });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), peer.outbound.recv())
            .await
            .is_err()
    );
    query.abort();
    assert!(query.await.unwrap_err().is_cancelled());
    let frames = [peer.next().await, peer.next().await];
    let cancel = frames
        .iter()
        .find(|frame| frame["method"] == "$/cancelRequest")
        .unwrap();
    assert_eq!(cancel["params"]["id"], hover["id"]);
    assert!(
        frames
            .iter()
            .any(|frame| frame["method"] == "textDocument/didChange")
    );
    assert!(peer.client.inner.pending.is_empty());
    queued.await.unwrap().unwrap();
}

#[tokio::test]
async fn accepts_pull_diagnostics_on_the_synchronized_version() {
    let mut peer = Peer::new();
    peer.initialize(json!({
        "textDocumentSync": 1,
        "diagnosticProvider": {"identifier": "test", "interFileDependencies": false, "workspaceDiagnostics": false},
    })).await.unwrap();
    let uri: Uri = "file:///project/file.rs".parse().unwrap();
    let receipt = peer
        .client
        .sync_document_with_receipt(uri.clone(), "fn main() {}".into(), "rust")
        .await
        .unwrap();
    assert_eq!(peer.next().await["method"], "textDocument/didOpen");
    let version = receipt.version;
    let client = Arc::clone(&peer.client);
    let pull = tokio::spawn(async move {
        client
            .document_diagnostics(&uri, receipt, Duration::from_secs(1))
            .await
    });
    let request = peer.next().await;
    assert_eq!(request["method"], "textDocument/diagnostic");
    assert_eq!(request["params"]["identifier"], "test");
    peer.send(json!({"jsonrpc":"2.0", "id":request["id"], "result":{"kind":"full","items":[]}}));
    let report = pull.await.unwrap().unwrap();
    assert_eq!(
        (report.state, report.expected_version, report.total),
        ("current", version, 0)
    );
}
