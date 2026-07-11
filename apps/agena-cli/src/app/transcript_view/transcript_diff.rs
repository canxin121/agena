#[derive(Debug, Clone, Default)]
pub(in crate::app) struct ApplyPatchDisplay {
    pub(super) changes: Vec<agena::message::FileChangeRecord>,
    pub(super) diff: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::app) struct DiffStats {
    pub(super) file_count: usize,
    pub(super) additions: usize,
    pub(super) deletions: usize,
    pub(super) renames: usize,
    pub(super) line_count: usize,
}

pub(in crate::app) fn apply_patch_details(
    details: &agena::message::ToolOutput,
) -> Option<ApplyPatchDisplay> {
    let changes: Vec<agena::message::FileChangeRecord> = details
        .payload
        .get("changes")
        .cloned()
        .and_then(|value| serde_json::from_value(serde_json::Value::from(value)).ok())
        .unwrap_or_default();
    let diff = details
        .payload
        .get("diff")
        .and_then(agena::message::StructuredValue::as_text)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    if details.payload.get("operation_id").is_none() && changes.is_empty() && diff.is_empty() {
        return None;
    }

    Some(ApplyPatchDisplay { changes, diff })
}

pub(in crate::app) fn diff_stats(
    diff: &str,
    changes: Option<&[agena::message::FileChangeRecord]>,
) -> DiffStats {
    let mut file_count = diff
        .lines()
        .filter(|line| line.starts_with("diff --git "))
        .count();
    let line_count = diff.lines().count();
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for line in diff.lines() {
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    let renames = if let Some(changes) = changes {
        file_count = file_count.max(changes.len());
        changes
            .iter()
            .filter(|change| change.kind == FileChangeKind::Moved)
            .count()
    } else {
        0
    };
    DiffStats {
        file_count,
        additions,
        deletions,
        renames,
        line_count,
    }
}

pub(in crate::app) fn file_change_display_path(
    change: &agena::message::FileChangeRecord,
) -> String {
    if change.kind == FileChangeKind::Moved {
        change
            .from_path
            .as_ref()
            .map(|from_path| format!("{from_path} -> {}", change.path))
            .unwrap_or_else(|| change.path.clone())
    } else {
        change.path.clone()
    }
}

pub(in crate::app) fn file_change_marker(kind: FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Added => "A",
        FileChangeKind::Updated => "M",
        FileChangeKind::Deleted => "D",
        FileChangeKind::Moved => "R",
    }
}

pub(in crate::app) fn file_change_style(kind: FileChangeKind) -> Style {
    match kind {
        FileChangeKind::Added => Style::default().fg(Color::Green),
        FileChangeKind::Updated => Style::default().fg(Color::Yellow),
        FileChangeKind::Deleted => Style::default().fg(Color::Red),
        FileChangeKind::Moved => Style::default().fg(Color::Cyan),
    }
}

pub(in crate::app) fn file_change_list_item_text(
    change: &agena::message::FileChangeRecord,
    i18n: &I18n,
) -> String {
    format!(
        "{} {} ({})",
        file_change_marker(change.kind),
        file_change_display_path(change),
        ui_text::file_change_kind_label(i18n, change.kind)
    )
}
use super::{Color, FileChangeKind, I18n, Style, ui_text};
