//! Diagnostic evidence belongs to one exact sent document version. Servers
//! that omit versions remain useful, but cannot certify current-version clean.
use super::*;

#[derive(Debug, Clone)]
pub struct DocumentReceipt {
    pub status: DocumentSyncStatus,
    pub version: i32,
}
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    pub expected_version: i32,
    pub reported_version: Option<i32>,
    pub state: &'static str,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Clone)]
pub(super) struct PublishedDiagnostics {
    pub version: Option<i32>,
    pub entries: Vec<Diagnostic>,
}

pub(super) struct SyncRollback {
    pub inner: std::sync::Arc<Inner>,
    pub uri: Uri,
    pub previous: Option<OpenDocState>,
    pub armed: bool,
}
impl Drop for SyncRollback {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(previous) = self.previous.take() {
            self.inner.open_docs.insert(self.uri.clone(), previous);
        } else {
            self.inner.open_docs.remove(&self.uri);
        }
        self.inner
            .diagnostics_changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

impl LspClient {
    pub fn diagnostic_report(&self, uri: &Uri, expected: i32) -> DiagnosticReport {
        let current = self.inner.open_docs.get(uri).map(|doc| doc.version);
        let published = self.inner.diagnostics.get(uri);
        let state = if current != Some(expected) {
            "superseded"
        } else {
            match published.as_ref() {
                Some(item) if item.version == Some(expected) => "current",
                Some(item) if item.version.is_none() => "unversioned",
                Some(_) => "stale",
                None => "pending",
            }
        };
        DiagnosticReport {
            expected_version: expected,
            reported_version: published.as_ref().and_then(|item| item.version),
            state,
            diagnostics: if state == "current" || state == "unversioned" {
                published
                    .map(|item| item.entries.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        }
    }
    pub async fn wait_diagnostics(
        &self,
        uri: &Uri,
        expected: i32,
        timeout: Duration,
    ) -> DiagnosticReport {
        let mut changed = self.inner.diagnostics_changed.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut report = self.diagnostic_report(uri, expected);
            if matches!(report.state, "current" | "unversioned" | "superseded") {
                return report;
            }
            if tokio::time::timeout_at(deadline, changed.changed())
                .await
                .is_err()
            {
                report.state = if report.state == "pending" {
                    "timeout"
                } else {
                    report.state
                };
                return report;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{InboundMessage, JSONRPC_VERSION, JsonRpcNotification},
        transport::InMemoryTransport,
    };
    fn publication(uri: &Uri, version: Option<i32>, message: Option<&str>) -> InboundMessage {
        let entries=message.map(|message|vec![serde_json::json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":message})]).unwrap_or_default();
        InboundMessage::Notification(JsonRpcNotification {
            jsonrpc: JSONRPC_VERSION.into(),
            method: "textDocument/publishDiagnostics".into(),
            params: Some(serde_json::json!({"uri":uri,"version":version,"diagnostics":entries})),
        })
    }
    #[tokio::test]
    async fn slow_publication_is_awaited_and_old_results_cannot_replace_newer_versions() {
        let (transport, mut outbound, inbound) = InMemoryTransport::pair();
        let client = LspClient::new(transport);
        let uri: Uri = "file:///fixture/a.rs".parse().unwrap();
        let one = client
            .sync_document_with_receipt(uri.clone(), "one".into(), "rust")
            .await
            .unwrap();
        outbound.recv().await.unwrap();
        assert_eq!(client.diagnostic_report(&uri, one.version).state, "pending");
        let sender = inbound.clone();
        let u = uri.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            sender
                .send(publication(&u, Some(one.version), Some("current error")))
                .await
                .unwrap();
        });
        let report = client
            .wait_diagnostics(&uri, one.version, Duration::from_secs(2))
            .await;
        assert_eq!(report.state, "current");
        assert_eq!(report.diagnostics.len(), 1);
        let two = client
            .sync_document_with_receipt(uri.clone(), "two".into(), "rust")
            .await
            .unwrap();
        outbound.recv().await.unwrap();
        assert_eq!(
            client.diagnostic_report(&uri, one.version).state,
            "superseded"
        );
        inbound
            .send(publication(&uri, Some(two.version), None))
            .await
            .unwrap();
        assert_eq!(
            client
                .wait_diagnostics(&uri, two.version, Duration::from_secs(1))
                .await
                .state,
            "current"
        );
        inbound
            .send(publication(&uri, Some(one.version), Some("stale error")))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        let report = client.diagnostic_report(&uri, two.version);
        assert_eq!(report.state, "current");
        assert!(report.diagnostics.is_empty());
    }
    #[tokio::test]
    async fn absent_or_unversioned_publication_never_certifies_clean() {
        let (transport, mut outbound, inbound) = InMemoryTransport::pair();
        let client = LspClient::new(transport);
        let uri: Uri = "file:///fixture/b.rs".parse().unwrap();
        let receipt = client
            .sync_document_with_receipt(uri.clone(), "text".into(), "rust")
            .await
            .unwrap();
        outbound.recv().await.unwrap();
        assert_eq!(
            client
                .wait_diagnostics(&uri, receipt.version, Duration::from_millis(10))
                .await
                .state,
            "timeout"
        );
        inbound.send(publication(&uri, None, None)).await.unwrap();
        assert_eq!(
            client
                .wait_diagnostics(&uri, receipt.version, Duration::from_secs(1))
                .await
                .state,
            "unversioned"
        );
    }
    #[tokio::test]
    async fn simultaneous_syncs_get_distinct_versions() {
        let (transport, _outbound, _inbound) = InMemoryTransport::pair();
        let client = LspClient::new(transport);
        let uri: Uri = "file:///fixture/c.rs".parse().unwrap();
        let (one, two) = tokio::join!(
            client.sync_document_with_receipt(uri.clone(), "one".into(), "rust"),
            client.sync_document_with_receipt(uri, "two".into(), "rust")
        );
        assert_ne!(one.unwrap().version, two.unwrap().version);
    }
}
