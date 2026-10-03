#[derive(Debug, Clone, Default)]
pub(crate) struct ApplyPatchDisplay {
    pub(super) changes: Vec<agena_domain::FileChangeRecord>,
    pub(super) diff: String,
}

pub(crate) fn apply_patch_details(details: &agena_domain::ToolOutput) -> Option<ApplyPatchDisplay> {
    let changes: Vec<agena_domain::FileChangeRecord> = match details.payload.get("changes") {
        Some(value) => match serde_json::from_value(serde_json::Value::from(value.clone())) {
            Ok(changes) => changes,
            Err(error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "decode apply-patch changes for transcript rendering",
                        &error,
                    ),
                    "apply-patch transcript details are incomplete"
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let diff = details
        .payload
        .get("diff")
        .and_then(agena_domain::StructuredValue::as_text)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    if details.payload.get("operation_id").is_none() && changes.is_empty() && diff.is_empty() {
        return None;
    }

    Some(ApplyPatchDisplay { changes, diff })
}

pub(crate) fn file_change_display_path(change: &agena_domain::FileChangeRecord) -> String {
    if change.kind == agena_domain::FileChangeKind::Moved {
        change
            .from_path
            .as_ref()
            .map(|from_path| format!("{from_path} -> {}", change.path))
            .unwrap_or_else(|| change.path.clone())
    } else {
        change.path.clone()
    }
}

pub(crate) fn file_change_marker(kind: agena_domain::FileChangeKind) -> &'static str {
    match kind {
        agena_domain::FileChangeKind::Added => "A",
        agena_domain::FileChangeKind::Updated => "M",
        agena_domain::FileChangeKind::Deleted => "D",
        agena_domain::FileChangeKind::Moved => "R",
    }
}

pub(crate) fn file_change_list_item_text(
    change: &agena_domain::FileChangeRecord,
    i18n: &I18n,
) -> String {
    format!(
        "{} {} ({})",
        file_change_marker(change.kind),
        file_change_display_path(change),
        match change.kind {
            agena_domain::FileChangeKind::Added => ui_text::t(i18n, "file-change-added"),
            agena_domain::FileChangeKind::Updated => ui_text::t(i18n, "file-change-updated"),
            agena_domain::FileChangeKind::Deleted => ui_text::t(i18n, "file-change-deleted"),
            agena_domain::FileChangeKind::Moved => ui_text::t(i18n, "file-change-moved"),
        }
    )
}
use super::I18n;
use crate::ui_text;
