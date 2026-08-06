use std::path::Path;

use agena_api::resource::{
    MessageRole, MessageStatus, PendingInteractiveRequest, SessionExecutionResource,
};
use agena_domain::PermissionReplyKind;
use agena_plugin_sdk::AttachmentKind;
use chrono::{DateTime, Local, Utc};

use agena_tui::{fl_args, i18n::I18n};

pub fn t(i18n: &I18n, key: &str) -> String {
    i18n.text(key)
}

pub fn thinking_mode_display_value(value: &str) -> String {
    value.trim().to_owned()
}

pub fn speed_mode_display_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("no-speed") {
        return "off".to_owned();
    }
    prefixed_mode_display_value(trimmed, &["speed-"])
}

fn prefixed_mode_display_value(value: &str, prefixes: &[&str]) -> String {
    prefixes
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .unwrap_or(value)
        .to_owned()
}

pub fn transcript_header_title(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
) -> String {
    match session_id {
        Some(id) => format!(" #{}  {} ", id, session_title),
        None => format!(" {} ", t(i18n, "pane-transcript")),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{I18n, text_artifact_display_label, transcript_header_title};

    #[test]
    fn session_header_places_id_before_title() {
        assert_eq!(
            transcript_header_title(&I18n::english(), Some(702), "New session 01:27"),
            " #702  New session 01:27 "
        );
    }

    #[test]
    fn text_artifact_label_uses_text_prefix_and_remaining_count() {
        assert_eq!(
            text_artifact_display_label("abcdefghijklmnopqrstuvwxyz", Some("paste 26 chars")),
            "abcdefghijkl… +14 chars"
        );
    }

    #[test]
    fn text_artifact_label_keeps_short_text_as_is() {
        assert_eq!(
            text_artifact_display_label("hello", Some("paste 5 chars")),
            "hello"
        );
    }

    #[test]
    fn text_artifact_label_falls_back_to_label_for_empty_text() {
        assert_eq!(
            text_artifact_display_label("", Some("paste 0 chars")),
            "paste 0 chars"
        );
    }
}

pub fn session_meta(i18n: &I18n, id: i64, message_count: u64, updated_at: DateTime<Utc>) -> String {
    i18n.text_args(
        "session-meta",
        &fl_args!(
            "id" => id,
            "message_count" => message_count,
            "updated" => format_relative_time(i18n, updated_at),
        ),
    )
}

pub fn transcript_search_summary(i18n: &I18n, query: &str, current: usize, total: usize) -> String {
    i18n.text_args(
        "transcript-header-find",
        &fl_args!(
            "query" => query,
            "current" => current as i64,
            "total" => total as i64,
        ),
    )
}

pub fn prefixed_query(prefix: &str, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{query}")
    }
}

pub fn format_relative_time(i18n: &I18n, timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(timestamp);
    if delta.num_seconds() < 60 {
        t(i18n, "time-just-now")
    } else if delta.num_minutes() < 60 {
        i18n.text_args(
            "time-minutes-ago",
            &fl_args!("count" => delta.num_minutes()),
        )
    } else if delta.num_hours() < 24 {
        i18n.text_args("time-hours-ago", &fl_args!("count" => delta.num_hours()))
    } else {
        i18n.text_args("time-days-ago", &fl_args!("count" => delta.num_days()))
    }
}

pub fn role_label(i18n: &I18n, role: MessageRole) -> String {
    match role {
        MessageRole::User => t(i18n, "message-role-user"),
        MessageRole::Assistant => t(i18n, "message-role-assistant"),
        MessageRole::System => t(i18n, "message-role-system"),
        MessageRole::Tool => "tool".to_string(),
    }
}

pub fn message_state_label(i18n: &I18n, state: MessageStatus) -> String {
    match state {
        MessageStatus::Pending => t(i18n, "message-state-pending"),
        MessageStatus::InProgress => t(i18n, "message-state-in-progress"),
        MessageStatus::Completed => t(i18n, "message-state-completed"),
        MessageStatus::PolicyDenied => t(i18n, "message-state-policy-denied"),
        MessageStatus::UserDeclined => t(i18n, "message-state-user-declined"),
        MessageStatus::CapabilityUnavailable => t(i18n, "message-state-capability-unavailable"),
        MessageStatus::ToolUnavailable => t(i18n, "message-state-tool-unavailable"),
        MessageStatus::Failed => t(i18n, "message-state-failed"),
        MessageStatus::Cancelled => t(i18n, "todo-status-cancelled"),
    }
}

pub fn permission_reply_label(i18n: &I18n, kind: PermissionReplyKind) -> String {
    match kind {
        PermissionReplyKind::AllowOnce => t(i18n, "permission-label-allow-once"),
        PermissionReplyKind::AllowAlways => t(i18n, "permission-label-allow-always"),
        PermissionReplyKind::DenyOnce => t(i18n, "permission-label-deny-once"),
        PermissionReplyKind::DenyAlways => t(i18n, "permission-label-deny-always"),
        PermissionReplyKind::AutoApprove => t(i18n, "permission-label-auto-approve"),
    }
}

pub fn default_session_title(i18n: &I18n) -> String {
    i18n.text_args(
        "session-default-title",
        &fl_args!("time" => Local::now().format("%H:%M").to_string()),
    )
}

pub fn session_fallback_title(i18n: &I18n, session_id: i64) -> String {
    i18n.text_args("session-fallback-title", &fl_args!("id" => session_id))
}

pub fn transcript_export_default_title(i18n: &I18n) -> String {
    t(i18n, "transcript-export-title")
}

pub fn transcript_export_session_id_line(i18n: &I18n, session_id: i64) -> String {
    i18n.text_args(
        "transcript-export-session-id",
        &fl_args!("id" => session_id),
    )
}

pub fn transcript_export_exported_at_line(i18n: &I18n, exported_at: DateTime<Local>) -> String {
    i18n.text_args(
        "transcript-export-exported-at",
        &fl_args!("time" => exported_at.format("%Y-%m-%d %H:%M:%S %z").to_string()),
    )
}

pub fn transcript_export_messages_loaded_line(i18n: &I18n, count: usize) -> String {
    i18n.text_args(
        "transcript-export-messages-loaded",
        &fl_args!("count" => count as i64),
    )
}

pub fn transcript_export_parent_session_line(i18n: &I18n, parent_id: i64) -> String {
    i18n.text_args(
        "transcript-export-parent-session",
        &fl_args!("id" => parent_id),
    )
}

pub fn transcript_export_child_sessions_line(i18n: &I18n, count: u64) -> String {
    i18n.text_args(
        "transcript-export-child-sessions",
        &fl_args!("count" => count as i64),
    )
}

pub fn transcript_export_empty_line(i18n: &I18n) -> String {
    t(i18n, "transcript-export-empty")
}

pub fn transcript_export_path_is_directory_error(i18n: &I18n, path: &Path) -> String {
    i18n.text_args(
        "transcript-export-path-is-directory",
        &fl_args!("path" => path.display()),
    )
}

pub fn no_session_selected_text(i18n: &I18n) -> String {
    [
        t(i18n, "no-session-selected"),
        t(i18n, "no-session-selected-hint"),
    ]
    .join("\n")
}

pub fn transcript_footer_plugin_block(i18n: &I18n, label: &str, body: &str) -> String {
    let label = label.trim();
    let body = body.trim();
    if label.is_empty() {
        body.to_string()
    } else {
        i18n.text_args(
            "transcript-footer-plugin-block",
            &fl_args!("label" => label, "body" => body),
        )
    }
}

pub fn session_workflow_state_label(i18n: &I18n, execution: &SessionExecutionResource) -> String {
    match execution
        .pending_interactive_requests
        .first()
        .map(|resource| &resource.request)
    {
        Some(PendingInteractiveRequest::Permission { .. }) => t(i18n, "session-awaiting-approval"),
        Some(PendingInteractiveRequest::UserInput { .. }) => t(i18n, "session-awaiting-user-input"),
        None if execution.workflow_state == agena_api::resource::WorkflowState::Blocked => {
            t(i18n, "session-blocked")
        }
        None if execution.active_execution.is_some() => t(i18n, "session-running"),
        None => t(i18n, "session-idle"),
    }
}

pub fn operation_search_heading(i18n: &I18n, query: Option<&str>) -> String {
    match query {
        Some(query) => i18n.text_args("operation-search-heading", &fl_args!("query" => query)),
        None => t(i18n, "operation-search-results-heading"),
    }
}

pub fn operation_command_exit_line(i18n: &I18n, exit_code: i32) -> String {
    i18n.text_args(
        "operation-command-exit-code",
        &fl_args!("code" => exit_code),
    )
}

pub fn operation_diff_summary(
    i18n: &I18n,
    file_count: usize,
    additions: usize,
    deletions: usize,
    renames: usize,
    line_count: usize,
) -> String {
    if file_count == 0 {
        return i18n.text_args(
            "operation-diff-summary",
            &fl_args!("count" => line_count as i64),
        );
    }

    let key = if file_count == 1 && renames > 0 {
        "operation-diff-summary-detailed-one-rename"
    } else if file_count == 1 {
        "operation-diff-summary-detailed-one"
    } else if renames > 0 {
        "operation-diff-summary-detailed-renames"
    } else {
        "operation-diff-summary-detailed"
    };

    i18n.text_args(
        key,
        &fl_args!(
            "files" => file_count as i64,
            "added" => additions as i64,
            "deleted" => deletions as i64,
            "renamed" => renames as i64,
        ),
    )
}

pub fn operation_nested_task_summary(
    i18n: &I18n,
    title: &str,
    status: agena_api::message_part::PartExecutionStatusResource,
) -> String {
    i18n.text_args(
        "operation-nested-task-summary",
        &fl_args!(
            "title" => title,
            "status" => match status {
                agena_api::message_part::PartExecutionStatusResource::Pending => t(i18n, "message-state-pending"),
                agena_api::message_part::PartExecutionStatusResource::InProgress => t(i18n, "message-state-in-progress"),
                agena_api::message_part::PartExecutionStatusResource::Completed => t(i18n, "message-state-completed"),
                agena_api::message_part::PartExecutionStatusResource::PolicyDenied => t(i18n, "message-state-policy-denied"),
                agena_api::message_part::PartExecutionStatusResource::UserDeclined => t(i18n, "message-state-user-declined"),
                agena_api::message_part::PartExecutionStatusResource::CapabilityUnavailable => t(i18n, "message-state-capability-unavailable"),
                agena_api::message_part::PartExecutionStatusResource::ToolUnavailable => t(i18n, "message-state-tool-unavailable"),
                agena_api::message_part::PartExecutionStatusResource::Failed => t(i18n, "message-state-failed"),
                agena_api::message_part::PartExecutionStatusResource::Cancelled => t(i18n, "todo-status-cancelled"),
            },
        ),
    )
}

pub fn message_error_text(i18n: &I18n, code: &str, message: &str) -> String {
    i18n.text_args(
        "message-error",
        &fl_args!("code" => code, "message" => message),
    )
}

pub fn message_permission_heading(i18n: &I18n) -> String {
    t(i18n, "message-permission")
}

pub fn snapshot_picker_title(i18n: &I18n) -> String {
    t(i18n, "overlay-snapshot-title")
}

pub fn snapshot_picker_prompt(i18n: &I18n) -> String {
    t(i18n, "command-snapshot-summary")
}

pub fn snapshot_ready_message(i18n: &I18n, path: &str, branch: Option<&str>) -> String {
    let branch = branch
        .map(str::to_owned)
        .unwrap_or_else(|| t(i18n, "value-unknown"));
    i18n.text_args(
        "flash-snapshot-ready",
        &fl_args!("path" => path, "branch" => branch),
    )
}

pub fn snapshot_attached_message(i18n: &I18n, path: &str, branch: Option<&str>) -> String {
    let branch = branch
        .map(str::to_owned)
        .unwrap_or_else(|| t(i18n, "value-unknown"));
    i18n.text_args(
        "flash-snapshot-attached",
        &fl_args!("path" => path, "branch" => branch),
    )
}

pub fn snapshot_exit_message(i18n: &I18n, action: Option<&str>, path: &str) -> String {
    i18n.text_args(
        "flash-snapshot-exited",
        &fl_args!(
            "action" => snapshot_action_label(i18n, action),
            "path" => path,
        ),
    )
}

pub fn commit_created_message(i18n: &I18n, sha: &str, summary: &str) -> String {
    i18n.text_args(
        "flash-commit-created",
        &fl_args!("sha" => sha, "summary" => summary),
    )
}

pub fn pull_request_created_message(i18n: &I18n, url: &str) -> String {
    i18n.text_args("flash-pr-created", &fl_args!("url" => url))
}

pub fn attachment_inspect_failed_message(i18n: &I18n, path: &Path, error: &str) -> String {
    i18n.text_args(
        "flash-attachment-inspect-failed",
        &fl_args!("path" => path.display(), "error" => error),
    )
}

pub fn composer_drafts_save_failed_message(i18n: &I18n, error: &str) -> String {
    i18n.text_args(
        "flash-composer-drafts-save-failed",
        &fl_args!("error" => error),
    )
}

pub fn prompt_history_save_failed_message(i18n: &I18n, error: &str) -> String {
    i18n.text_args(
        "flash-prompt-history-save-failed",
        &fl_args!("error" => error),
    )
}

pub fn composer_placeholder_range_invalid_error(i18n: &I18n) -> String {
    t(i18n, "composer-placeholder-range-invalid")
}

pub fn composer_placeholder_out_of_sync_error(i18n: &I18n) -> String {
    t(i18n, "composer-placeholder-out-of-sync")
}

pub fn composer_missing_staged_item_error(i18n: &I18n, placeholder: &str) -> String {
    i18n.text_args(
        "composer-missing-staged-item",
        &fl_args!("placeholder" => placeholder),
    )
}

pub fn message_question_line(i18n: &I18n, question: &str, id: &str) -> String {
    format!(
        "    {}",
        i18n.text_args(
            "message-question-line",
            &fl_args!("question" => question, "id" => id),
        )
    )
}

pub fn attachment_kind_label(i18n: &I18n, kind: AttachmentKind) -> String {
    match kind {
        AttachmentKind::Image => t(i18n, "attachment-kind-image"),
        AttachmentKind::Audio => t(i18n, "attachment-kind-audio"),
        AttachmentKind::Video => t(i18n, "attachment-kind-video"),
        AttachmentKind::Pdf => t(i18n, "attachment-kind-pdf"),
        AttachmentKind::File => t(i18n, "attachment-kind-file"),
    }
}

pub fn attachment_display_kind_label(
    i18n: &I18n,
    kind: AttachmentKind,
    is_directory: bool,
) -> String {
    if is_directory {
        t(i18n, "attachment-kind-directory")
    } else {
        attachment_kind_label(i18n, kind)
    }
}

pub fn attachment_chip_label(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    is_directory: bool,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: u64,
) -> String {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let kind_label = attachment_display_kind_label(i18n, kind, is_directory);
    let size = format_bytes(i18n, size_bytes);

    match (kind, width, height) {
        (AttachmentKind::Image, Some(width), Some(height)) => i18n.text_args(
            "attachment-chip-image",
            &fl_args!(
                "kind" => kind_label,
                "filename" => filename,
                "width" => width as i64,
                "height" => height as i64,
                "size" => size,
            ),
        ),
        _ => i18n.text_args(
            "attachment-chip-other",
            &fl_args!(
                "kind" => kind_label,
                "filename" => filename,
                "size" => size,
            ),
        ),
    }
}

/// Maximum length of a pasted-text artifact's inline placeholder. Large
/// pastes can carry a long persisted summary (the message-part summary is
/// truncated to 240 chars); showing that whole string as the inline
/// `[placeholder]` overflows the composer and transcript rows. Display a
/// compact prefix of the actual pasted text plus the remaining character
/// count (e.g. `abc… +997 chars`) so the placeholder identifies the content
/// instead of a generic `paste N chars` label.
pub fn text_artifact_display_label(text: &str, label: Option<&str>) -> String {
    const MAX_PLACEHOLDER_CHARS: usize = 24;
    const TEXT_PREFIX_CHARS: usize = 12;
    let count = text.chars().count();
    if count > TEXT_PREFIX_CHARS {
        let prefix: String = text.chars().take(TEXT_PREFIX_CHARS).collect();
        return format!("{prefix}… +{} chars", count - TEXT_PREFIX_CHARS);
    }
    if count > 0 {
        return text.to_owned();
    }
    let base = label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "pasted text".to_owned());
    let mut chars = base.chars();
    let mut truncated: String = chars.by_ref().take(MAX_PLACEHOLDER_CHARS).collect();
    if chars.next().is_some() {
        truncated.push('…');
    }
    truncated
}

pub fn attachment_placeholder_base(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    is_directory: bool,
) -> String {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| t(i18n, "attachment-generic"));
    let kind_label = attachment_display_kind_label(i18n, kind, is_directory);
    i18n.text_args(
        "attachment-placeholder",
        &fl_args!("kind" => kind_label, "filename" => filename),
    )
}

pub fn format_bytes(i18n: &I18n, bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let bytes_f = bytes as f64;
    if bytes_f >= GB {
        i18n.text_args(
            "bytes-gb",
            &fl_args!("value" => format!("{:.1}", bytes_f / GB)),
        )
    } else if bytes_f >= MB {
        i18n.text_args(
            "bytes-mb",
            &fl_args!("value" => format!("{:.1}", bytes_f / MB)),
        )
    } else if bytes_f >= KB {
        i18n.text_args(
            "bytes-kb",
            &fl_args!("value" => format!("{:.1}", bytes_f / KB)),
        )
    } else {
        i18n.text_args("bytes-b", &fl_args!("value" => bytes as i64))
    }
}

fn snapshot_action_label(i18n: &I18n, action: Option<&str>) -> String {
    match action.unwrap_or_default().to_ascii_lowercase().as_str() {
        "" => t(i18n, "snapshot-action-exit"),
        "keep" => t(i18n, "snapshot-action-keep"),
        "remove" => t(i18n, "snapshot-action-remove"),
        other => other.to_string(),
    }
}
