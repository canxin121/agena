#[derive(Debug, Clone, Default)]
pub(crate) struct ApplyPatchDisplay {
    pub(super) changes: Vec<agena_api::message_part::FileChangeRecordResource>,
    pub(super) diff: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DiffStats {
    pub(super) file_count: usize,
    pub(super) additions: usize,
    pub(super) deletions: usize,
    pub(super) renames: usize,
    pub(super) line_count: usize,
}

pub(crate) fn apply_patch_details(
    details: &agena_api::message_part::ToolOutputResource,
) -> Option<ApplyPatchDisplay> {
    let changes: Vec<agena_api::message_part::FileChangeRecordResource> = details
        .payload
        .get("changes")
        .cloned()
        .and_then(|value| serde_json::from_value(serde_json::Value::from(value)).ok())
        .unwrap_or_default();
    let diff = details
        .payload
        .get("diff")
        .and_then(agena_api::message_part::StructuredValueResource::as_text)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    if details.payload.get("operation_id").is_none() && changes.is_empty() && diff.is_empty() {
        return None;
    }

    Some(ApplyPatchDisplay { changes, diff })
}

pub(crate) fn diff_stats(
    diff: &str,
    changes: Option<&[agena_api::message_part::FileChangeRecordResource]>,
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
            .filter(|change| change.kind == agena_api::message_part::FileChangeKindResource::Moved)
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

pub(crate) fn file_change_display_path(
    change: &agena_api::message_part::FileChangeRecordResource,
) -> String {
    if change.kind == agena_api::message_part::FileChangeKindResource::Moved {
        change
            .from_path
            .as_ref()
            .map(|from_path| format!("{from_path} -> {}", change.path))
            .unwrap_or_else(|| change.path.clone())
    } else {
        change.path.clone()
    }
}

pub(crate) fn file_change_marker(
    kind: agena_api::message_part::FileChangeKindResource,
) -> &'static str {
    match kind {
        agena_api::message_part::FileChangeKindResource::Added => "A",
        agena_api::message_part::FileChangeKindResource::Updated => "M",
        agena_api::message_part::FileChangeKindResource::Deleted => "D",
        agena_api::message_part::FileChangeKindResource::Moved => "R",
    }
}

pub(crate) fn file_change_list_item_text(
    change: &agena_api::message_part::FileChangeRecordResource,
    i18n: &I18n,
) -> String {
    format!(
        "{} {} ({})",
        file_change_marker(change.kind),
        file_change_display_path(change),
        match change.kind {
            agena_api::message_part::FileChangeKindResource::Added =>
                ui_text::t(i18n, "file-change-added"),
            agena_api::message_part::FileChangeKindResource::Updated =>
                ui_text::t(i18n, "file-change-updated"),
            agena_api::message_part::FileChangeKindResource::Deleted =>
                ui_text::t(i18n, "file-change-deleted"),
            agena_api::message_part::FileChangeKindResource::Moved =>
                ui_text::t(i18n, "file-change-moved"),
        }
    )
}
use super::I18n;
use crate::ui_text;
