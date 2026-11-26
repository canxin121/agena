//! Match diagnostics to the snapshot sent for the request.

use super::*;
use lsp_types::{
    DiagnosticServerCapabilities, DocumentDiagnosticParams, DocumentDiagnosticReport,
    DocumentDiagnosticReportResult,
};

const MAX_CACHED_DIAGNOSTICS: usize = 128;
const MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 4096;

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
    pub total: usize,
    pub truncated: bool,
}

pub(super) struct PublishedDiagnostics {
    pub version: Option<i32>,
    pub sequence: u64,
    entries: Vec<Diagnostic>,
    total: usize,
    truncated: bool,
}

impl PublishedDiagnostics {
    pub(super) fn new(version: Option<i32>, sequence: u64, mut entries: Vec<Diagnostic>) -> Self {
        let total = entries.len();
        entries.truncate(MAX_CACHED_DIAGNOSTICS);
        let mut truncated = total > entries.len();
        for entry in &mut entries {
            if entry.message.len() > MAX_DIAGNOSTIC_MESSAGE_BYTES {
                entry.message.truncate(
                    entry
                        .message
                        .floor_char_boundary(MAX_DIAGNOSTIC_MESSAGE_BYTES),
                );
                truncated = true;
            }
            entry.data = None;
            entry.related_information = None;
        }
        Self {
            version,
            sequence,
            entries,
            total,
            truncated,
        }
    }

    fn report(&self, expected_version: i32, state: &'static str) -> DiagnosticReport {
        DiagnosticReport {
            expected_version,
            reported_version: self.version,
            state,
            diagnostics: self.entries.clone(),
            total: self.total,
            truncated: self.truncated,
        }
    }
}

impl LspClient {
    pub fn diagnostic_report(&self, uri: &Uri, expected: i32) -> DiagnosticReport {
        let empty = |state, reported_version| DiagnosticReport {
            expected_version: expected,
            reported_version,
            state,
            diagnostics: Vec::new(),
            total: 0,
            truncated: false,
        };
        if !self.is_alive() {
            return empty("disconnected", None);
        }
        let current = self
            .inner
            .open_docs
            .get(uri)
            .map(|doc| (doc.version, doc.publication_at_sync));
        let published = self.inner.diagnostics.get(uri);
        let reported = published.as_ref().and_then(|item| item.version);
        let Some((version, sequence)) = current.filter(|(version, _)| *version == expected) else {
            return empty("superseded", reported);
        };
        match published.as_ref() {
            Some(item) if item.version == Some(version) => item.report(expected, "current"),
            Some(item) if item.version.is_none() && item.sequence > sequence => {
                item.report(expected, "unversioned")
            }
            Some(_) => empty("stale", reported),
            None => empty("pending", None),
        }
    }

    /// Read diagnostics via pull when advertised, falling back to push for older peers.
    pub async fn document_diagnostics(
        &self,
        uri: &Uri,
        receipt: DocumentReceipt,
        wait: Duration,
    ) -> LspResult<DiagnosticReport> {
        let observed = self.diagnostic_report(uri, receipt.version);
        if matches!(observed.state, "superseded" | "disconnected") {
            return Ok(observed);
        }
        if let Some(server) = self.server_info()
            && let Some(provider) = server.capabilities.diagnostic_provider.as_ref()
        {
            let identifier = match provider {
                DiagnosticServerCapabilities::Options(options) => &options.identifier,
                DiagnosticServerCapabilities::RegistrationOptions(options) => {
                    &options.diagnostic_options.identifier
                }
            };
            let result = self
                .request::<_, DocumentDiagnosticReportResult>(
                    "textDocument/diagnostic",
                    DocumentDiagnosticParams {
                        text_document: TextDocumentIdentifier { uri: uri.clone() },
                        identifier: identifier.clone(),
                        previous_result_id: None,
                        work_done_progress_params: WorkDoneProgressParams::default(),
                        partial_result_params: PartialResultParams::default(),
                    },
                )
                .await;
            match result {
                Ok(DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(
                    full,
                ))) => {
                    self.ensure_alive()?;
                    let report = PublishedDiagnostics::new(
                        None,
                        0,
                        full.full_document_diagnostic_report.items,
                    );
                    let state = if self.syncs_open_documents() {
                        "current"
                    } else {
                        "unversioned"
                    };
                    return Ok(report.report(receipt.version, state));
                }
                Err(LspError::Server { code: -32601, .. }) => {}
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(LspError::Protocol(
                        "LSP pull diagnostics did not return a full report".into(),
                    ));
                }
            }
        }
        Ok(self.wait_diagnostics(uri, receipt.version, wait).await)
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
            let report = self.diagnostic_report(uri, expected);
            if !matches!(report.state, "pending" | "stale") {
                return report;
            }
            if tokio::time::timeout_at(deadline, changed.changed())
                .await
                .is_err()
            {
                let mut report = self.diagnostic_report(uri, expected);
                if report.state == "pending" {
                    report.state = "timeout";
                }
                return report;
            }
        }
    }
}
