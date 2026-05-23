use std::path::Path;

use agena::{
    message::{
        AttachmentKind, ExecutionStatus, FileChangeKind, MessageStatus, PermissionRequestPart,
        TodoPriority, TodoStatus,
    },
    permission::PermissionReplyKind,
};
use agena_api::resource::MessageRole;
use chrono::{DateTime, Local, Utc};

use crate::{fl_args, i18n::I18n};

pub fn t(i18n: &I18n, key: &str) -> String {
    i18n.text(key)
}

pub fn transcript_header_title(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    _is_running: bool,
) -> String {
    match session_id {
        Some(id) => format!(" {}  #{} ", session_title, id),
        None => format!(" {} ", t(i18n, "pane-transcript")),
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
    }
}

pub fn message_state_label(i18n: &I18n, state: MessageStatus) -> String {
    match state {
        MessageStatus::Pending => t(i18n, "message-state-pending"),
        MessageStatus::InProgress => t(i18n, "message-state-in-progress"),
        MessageStatus::Completed => t(i18n, "message-state-completed"),
        MessageStatus::Failed => t(i18n, "message-state-failed"),
        MessageStatus::Cancelled => t(i18n, "todo-status-cancelled"),
    }
}

pub fn execution_status_label(i18n: &I18n, status: ExecutionStatus) -> String {
    match status {
        ExecutionStatus::Pending => t(i18n, "message-state-pending"),
        ExecutionStatus::InProgress => t(i18n, "message-state-in-progress"),
        ExecutionStatus::Completed => t(i18n, "message-state-completed"),
        ExecutionStatus::Failed => t(i18n, "message-state-failed"),
        ExecutionStatus::Cancelled => t(i18n, "todo-status-cancelled"),
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
        None => {
            let mut summary = i18n.text_args(
                "permission-summary-pending",
                &fl_args!("reason" => permission.request.reason.as_str()),
            );
            if !permission.request.explanation.trim().is_empty() {
                summary.push_str(format!(" — {}", permission.request.explanation).as_str());
            }
            summary
        }
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

pub fn session_fallback_title(i18n: &I18n, session_id: i64) -> String {
    i18n.text_args("session-fallback-title", &fl_args!("id" => session_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpLineKind {
    Header,
    Section,
    Body,
    Spacer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpLine {
    pub text: String,
    pub kind: HelpLineKind,
}

pub fn help_lines(i18n: &I18n) -> Vec<HelpLine> {
    vec![
        HelpLine {
            text: t(i18n, "help-header"),
            kind: HelpLineKind::Header,
        },
        HelpLine {
            text: t(i18n, "status-global"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "status-transcript"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "status-composer"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: String::new(),
            kind: HelpLineKind::Spacer,
        },
        HelpLine {
            text: t(i18n, "help-section-sessions"),
            kind: HelpLineKind::Section,
        },
        HelpLine {
            text: t(i18n, "help-sessions-line-1"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-sessions-line-2"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-sessions-line-3"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-sessions-line-4"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-sessions-line-5"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: String::new(),
            kind: HelpLineKind::Spacer,
        },
        HelpLine {
            text: t(i18n, "help-section-transcript"),
            kind: HelpLineKind::Section,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-1"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-2"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-3"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-4"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-5"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-6"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-7"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-transcript-line-8"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: String::new(),
            kind: HelpLineKind::Spacer,
        },
        HelpLine {
            text: t(i18n, "help-section-composer"),
            kind: HelpLineKind::Section,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-1"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-2"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-3"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-4"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-5"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-6"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-7"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-8"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-9"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-10"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-composer-line-11"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: String::new(),
            kind: HelpLineKind::Spacer,
        },
        HelpLine {
            text: t(i18n, "help-section-actions"),
            kind: HelpLineKind::Section,
        },
        HelpLine {
            text: t(i18n, "help-actions-line-1"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-actions-line-2"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-actions-line-3"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-actions-line-4"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-actions-line-5"),
            kind: HelpLineKind::Body,
        },
        HelpLine {
            text: t(i18n, "help-actions-line-6"),
            kind: HelpLineKind::Body,
        },
    ]
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
