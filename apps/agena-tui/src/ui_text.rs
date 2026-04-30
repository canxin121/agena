use std::path::Path;

use agena::{
    message::{
        AttachmentKind, ExecutionStatus, FileChangeKind, MessageStatus, PermissionRequestPart,
        TodoPriority, TodoStatus,
    },
    permission::PermissionReplyKind,
    role::Role,
};
use chrono::{DateTime, Local, Utc};

use crate::{fl_args, i18n::I18n};

pub fn t(i18n: &I18n, key: &str) -> String {
    i18n.text(key)
}

pub fn sessions_title(i18n: &I18n, mode: &str, query: &str) -> String {
    let mut label = t(i18n, "pane-sessions");
    if !mode.trim().is_empty() {
        label.push_str(format!(" [{mode}]").as_str());
    }
    if !query.trim().is_empty() {
        label.push_str(format!(" [{}]", query.trim()).as_str());
    }
    format!(" {label} ")
}

pub fn transcript_panel_title(i18n: &I18n) -> String {
    format!(" {} ", t(i18n, "pane-messages"))
}

pub fn transcript_header_title(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    is_running: bool,
) -> String {
    match session_id {
        Some(id) => {
            let mut value = format!(" {} (#{id}", session_title);
            if is_running {
                value.push_str(format!(", {}", t(i18n, "session-running")).as_str());
            }
            value.push_str(") ");
            value
        }
        None => format!(" {} ", t(i18n, "pane-transcript")),
    }
}

pub fn composer_title(i18n: &I18n, session_id: Option<i64>) -> String {
    let session = session_id
        .map(|id| format!("#{id}"))
        .unwrap_or_else(|| t(i18n, "composer-session-new"));
    format!(
        " {} ",
        i18n.text_args("pane-composer", &fl_args!("session" => session))
    )
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

pub fn transcript_lines_summary(
    i18n: &I18n,
    first: usize,
    last: usize,
    total: usize,
    percent: u16,
) -> String {
    i18n.text_args(
        "transcript-header-lines",
        &fl_args!(
            "first" => first as i64,
            "last" => last as i64,
            "total" => total as i64,
            "percent" => percent as i64,
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

pub fn role_label(i18n: &I18n, role: Role) -> String {
    match role {
        Role::User => t(i18n, "message-role-user"),
        Role::Assistant => t(i18n, "message-role-assistant"),
        Role::System => t(i18n, "message-role-system"),
        Role::Tool => t(i18n, "message-role-tool"),
    }
}

pub fn message_state_label(i18n: &I18n, state: MessageStatus) -> String {
    match state {
        MessageStatus::Pending => t(i18n, "message-state-pending"),
        MessageStatus::InProgress => t(i18n, "message-state-in-progress"),
        MessageStatus::Completed => t(i18n, "message-state-completed"),
        MessageStatus::Failed => t(i18n, "message-state-failed"),
    }
}

pub fn execution_status_label(i18n: &I18n, status: ExecutionStatus) -> String {
    match status {
        ExecutionStatus::Pending => t(i18n, "message-state-pending"),
        ExecutionStatus::InProgress => t(i18n, "message-state-in-progress"),
        ExecutionStatus::Completed => t(i18n, "message-state-completed"),
        ExecutionStatus::Failed => t(i18n, "message-state-failed"),
    }
}

pub fn todo_status_label(i18n: &I18n, status: TodoStatus) -> String {
    match status {
        TodoStatus::Pending => t(i18n, "todo-status-pending"),
        TodoStatus::InProgress => t(i18n, "todo-status-in-progress"),
        TodoStatus::Completed => t(i18n, "todo-status-completed"),
        TodoStatus::Cancelled => t(i18n, "todo-status-cancelled"),
    }
}

pub fn todo_priority_label(i18n: &I18n, priority: TodoPriority) -> String {
    match priority {
        TodoPriority::High => t(i18n, "todo-priority-high"),
        TodoPriority::Medium => t(i18n, "todo-priority-medium"),
        TodoPriority::Low => t(i18n, "todo-priority-low"),
    }
}

pub fn file_change_kind_label(i18n: &I18n, kind: FileChangeKind) -> String {
    match kind {
        FileChangeKind::Added => t(i18n, "file-change-added"),
        FileChangeKind::Updated => t(i18n, "file-change-updated"),
        FileChangeKind::Deleted => t(i18n, "file-change-deleted"),
        FileChangeKind::Moved => "moved".to_string(),
    }
}

pub fn permission_reply_label(i18n: &I18n, kind: PermissionReplyKind) -> String {
    match kind {
        PermissionReplyKind::AllowOnce => t(i18n, "permission-label-allow-once"),
        PermissionReplyKind::AllowAlways => t(i18n, "permission-label-allow-always"),
        PermissionReplyKind::DenyOnce => t(i18n, "permission-label-deny-once"),
        PermissionReplyKind::DenyAlways => t(i18n, "permission-label-deny-always"),
    }
}

pub fn permission_summary(i18n: &I18n, permission: &PermissionRequestPart) -> String {
    match permission.reply.as_ref() {
        None => i18n.text_args(
            "permission-summary-pending",
            &fl_args!("reason" => permission.request.reason.as_str()),
        ),
        Some(reply) => {
            let reason = reply
                .reason
                .as_deref()
                .unwrap_or(permission.request.reason.as_str());
            let key = match reply.kind {
                PermissionReplyKind::AllowOnce => "permission-summary-allow-once",
                PermissionReplyKind::AllowAlways => "permission-summary-allow-always",
                PermissionReplyKind::DenyOnce => "permission-summary-deny-once",
                PermissionReplyKind::DenyAlways => "permission-summary-deny-always",
            };
            i18n.text_args(key, &fl_args!("reason" => reason))
        }
    }
}

pub fn default_session_title(i18n: &I18n) -> String {
    i18n.text_args(
        "session-default-title",
        &fl_args!("time" => Local::now().format("%H:%M").to_string()),
    )
}

#[allow(dead_code)]
pub fn empty_session_title(i18n: &I18n) -> String {
    t(i18n, "session-default-base")
}

pub fn session_fallback_title(i18n: &I18n, session_id: i64) -> String {
    i18n.text_args("session-fallback-title", &fl_args!("id" => session_id))
}

pub fn user_input_error_empty(i18n: &I18n) -> String {
    t(i18n, "user-input-error-empty")
}

pub fn user_input_error_invalid_segment(i18n: &I18n, segment: &str) -> String {
    i18n.text_args(
        "user-input-error-invalid-segment",
        &fl_args!("segment" => segment),
    )
}

pub fn user_input_error_unknown_question(i18n: &I18n, question_id: &str) -> String {
    i18n.text_args(
        "user-input-error-unknown-question",
        &fl_args!("question_id" => question_id),
    )
}

pub fn user_input_error_missing_answer(i18n: &I18n, question_id: &str) -> String {
    i18n.text_args(
        "user-input-error-missing-answer",
        &fl_args!("question_id" => question_id),
    )
}

pub fn user_input_error_no_answers(i18n: &I18n) -> String {
    t(i18n, "user-input-error-no-answers")
}

pub fn help_lines(i18n: &I18n) -> Vec<String> {
    vec![
        t(i18n, "help-header"),
        String::new(),
        t(i18n, "help-section-sessions"),
        t(i18n, "help-sessions-line-1"),
        t(i18n, "help-sessions-line-2"),
        t(i18n, "help-sessions-line-3"),
        t(i18n, "help-sessions-line-4"),
        t(i18n, "help-sessions-line-5"),
        String::new(),
        t(i18n, "help-section-transcript"),
        t(i18n, "help-transcript-line-1"),
        t(i18n, "help-transcript-line-2"),
        t(i18n, "help-transcript-line-3"),
        t(i18n, "help-transcript-line-4"),
        t(i18n, "help-transcript-line-5"),
        t(i18n, "help-transcript-line-6"),
        t(i18n, "help-transcript-line-7"),
        t(i18n, "help-transcript-line-8"),
        String::new(),
        t(i18n, "help-section-composer"),
        t(i18n, "help-composer-line-1"),
        t(i18n, "help-composer-line-2"),
        t(i18n, "help-composer-line-3"),
        t(i18n, "help-composer-line-4"),
        t(i18n, "help-composer-line-5"),
        t(i18n, "help-composer-line-6"),
        t(i18n, "help-composer-line-7"),
        t(i18n, "help-composer-line-8"),
        t(i18n, "help-composer-line-9"),
        t(i18n, "help-composer-line-10"),
        String::new(),
        t(i18n, "help-section-actions"),
        t(i18n, "help-actions-line-1"),
        t(i18n, "help-actions-line-2"),
        t(i18n, "help-actions-line-3"),
        t(i18n, "help-actions-line-4"),
        t(i18n, "help-actions-line-5"),
        t(i18n, "help-actions-line-6"),
    ]
}

pub fn message_usage(i18n: &I18n, input: u64, output: u64, reasoning: u64) -> String {
    format!(
        "  {}",
        i18n.text_args(
            "message-usage",
            &fl_args!(
                "input" => input,
                "output" => output,
                "reasoning" => reasoning,
            ),
        )
    )
}

pub fn message_finish(i18n: &I18n, finish: &str) -> String {
    format!(
        "  {}",
        i18n.text_args("message-finish", &fl_args!("finish" => finish))
    )
}

pub fn message_parts_not_loaded(i18n: &I18n, count: usize) -> String {
    format!(
        "  {}",
        i18n.text_args(
            "message-parts-not-loaded",
            &fl_args!("count" => count as i64),
        )
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

pub fn message_tool_result_blocks(i18n: &I18n, count: usize) -> String {
    format!(
        "    {}",
        i18n.text_args(
            "message-tool-result-blocks",
            &fl_args!("count" => count as i64),
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

pub fn attachment_chip_label(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: u64,
) -> String {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let kind_label = attachment_kind_label(i18n, kind);
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

pub fn attachment_placeholder_base(i18n: &I18n, path: &Path, kind: AttachmentKind) -> String {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| t(i18n, "attachment-generic"));
    let kind_label = attachment_kind_label(i18n, kind);
    i18n.text_args(
        "attachment-placeholder",
        &fl_args!("kind" => kind_label, "filename" => filename),
    )
}

pub fn staged_paste_label(i18n: &I18n, count: usize, append_on_send: bool) -> String {
    let key = if append_on_send {
        "paste-label-append"
    } else {
        "paste-label"
    };
    i18n.text_args(key, &fl_args!("count" => count as i64))
}

pub fn staged_paste_placeholder(i18n: &I18n, count: usize) -> String {
    i18n.text_args("paste-placeholder", &fl_args!("count" => count as i64))
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
