use std::path::Path;

use agena_lsp::DiagnosticReport;
use agena_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, SymbolKind, Uri,
};
use serde::{Deserialize, Serialize};

use super::ToolExecutor;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LspSource {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub server: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source_revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_version: Option<i32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub position_encoding: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspTarget {
    pub uri: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_range: Option<Range>,
}

impl LspTarget {
    pub(super) fn new(
        executor: &ToolExecutor,
        uri: &Uri,
        range: Option<Range>,
        selection_range: Option<Range>,
    ) -> Self {
        let path = url::Url::parse(uri.as_str())
            .ok()
            .and_then(|uri| uri.to_file_path().ok())
            .map_or_else(|| uri.to_string(), |path| executor.display_path(&path));
        Self {
            uri: uri.to_string(),
            path,
            range,
            selection_range,
        }
    }

    pub(super) fn label(&self) -> String {
        let Some(range) = self.selection_range.or(self.range) else {
            return self.path.clone();
        };
        let file_path = url::Url::parse(&self.uri)
            .ok()
            .and_then(|uri| uri.to_file_path().ok());
        let path = file_path.as_deref().map(Path::to_string_lossy);
        position_label(path.as_deref().unwrap_or(&self.path), range.start)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LspLocationsResult {
    pub locations: Vec<String>,
    pub targets: Vec<LspTarget>,
    pub total_locations: usize,
    pub truncated: bool,
    #[serde(flatten)]
    pub source: LspSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LspHoverResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    pub truncated: bool,
    #[serde(flatten)]
    pub source: LspSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub range: Range,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<NumberOrString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LspDiagnosticResult {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    pub entries: Vec<String>,
    pub diagnostics: Vec<LspDiagnostic>,
    pub total_diagnostics: usize,
    pub truncated: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub source: LspSource,
}

impl LspDiagnosticResult {
    pub(super) fn from_report(
        executor: &ToolExecutor,
        path: &Path,
        report: DiagnosticReport,
        source: LspSource,
        budget: usize,
    ) -> Self {
        let mut result = Self {
            file: executor.display_path(path),
            state: report.state.into(),
            total_diagnostics: report.total,
            truncated: report.truncated,
            reported_version: report.reported_version,
            source,
            ..Self::default()
        };
        let mut remaining = budget;
        for diagnostic in report.diagnostics {
            let mut item = LspDiagnostic::from(diagnostic);
            let message_limit = budget / 4;
            if item.message.len() > message_limit {
                item.message
                    .truncate(item.message.floor_char_boundary(message_limit));
                result.truncated = true;
            }
            let label = format!(
                "{} [{}] {}",
                position_label(&result.file, item.range.start),
                item.severity,
                item.message,
            );
            if !take_budget(&mut remaining, &(&item, &label)) {
                result.truncated = true;
                break;
            }
            result.diagnostics.push(item);
            result.entries.push(label);
        }
        result
    }
}

impl From<Diagnostic> for LspDiagnostic {
    fn from(diagnostic: Diagnostic) -> Self {
        let severity = match diagnostic.severity {
            Some(DiagnosticSeverity::ERROR) => "error",
            Some(DiagnosticSeverity::WARNING) => "warning",
            Some(DiagnosticSeverity::INFORMATION) => "info",
            Some(DiagnosticSeverity::HINT) => "hint",
            _ => "note",
        };
        Self {
            range: diagnostic.range,
            severity: severity.into(),
            message: diagnostic.message.trim().to_owned(),
            code: diagnostic.code,
            source: diagnostic.source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspSymbol {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(flatten)]
    pub target: LspTarget,
}

pub(super) fn position_label(path: &str, position: Position) -> String {
    format!(
        "{}:{}:{}",
        path,
        u64::from(position.line) + 1,
        u64::from(position.character) + 1
    )
}

pub(in crate::tool) fn preview(text: &str, bytes: usize) -> &str {
    &text[..text.floor_char_boundary(bytes.min(text.len()))]
}

pub(super) fn take_budget(remaining: &mut usize, item: &impl Serialize) -> bool {
    let bytes = serde_json::to_vec(item)
        .expect("LSP display values serialize")
        .len();
    if let Some(next) = remaining.checked_sub(bytes) {
        *remaining = next;
        true
    } else {
        false
    }
}
