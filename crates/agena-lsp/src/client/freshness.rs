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
