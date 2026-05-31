use std::path::Path;

use agena::{
    message::{
        AttachmentKind, ExecutionStatus, FileChangeKind, InteractiveRequestPart, MessageStatus,
        TodoPriority, TodoStatus,
    },
    permission::{PermissionReply, PermissionReplyKind, PermissionRequest},
};
use agena_api::resource::{
    MessageRole, PendingInteractiveRequest, SessionExecutionResource, SessionRunState,
};
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
        FileChangeKind::Moved => t(i18n, "file-change-moved"),
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

pub fn permission_summary(
    i18n: &I18n,
    permission: &InteractiveRequestPart<PermissionRequest, PermissionReply>,
) -> String {
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

pub fn transcript_export_older_messages_omitted_line(i18n: &I18n, has_more_older: bool) -> String {
    i18n.text_args(
        "transcript-export-older-messages-omitted",
        &fl_args!(
            "value" => t(i18n, if has_more_older { "value-yes" } else { "value-no" }),
        ),
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
    match execution.pending_interactive_requests.first() {
        Some(PendingInteractiveRequest::Permission { request })
            if execution.plan.is_some() && permission_request_is_plan_approval(request) =>
        {
            t(i18n, "session-awaiting-plan-approval")
        }
        Some(PendingInteractiveRequest::Permission { .. }) => t(i18n, "session-awaiting-approval"),
        Some(PendingInteractiveRequest::UserInput { .. }) => t(i18n, "session-awaiting-user-input"),
        None if execution.blocked => t(i18n, "session-blocked"),
        None => match execution.run_state {
            SessionRunState::AwaitingModel => t(i18n, "session-awaiting-model"),
            SessionRunState::Idle => t(i18n, "session-idle"),
        },
    }
}

fn permission_request_is_plan_approval(request: &PermissionRequest) -> bool {
    matches!(
        &request.action,
        agena::permission::PermissionAction::Tool {
            tool_name,
            qualifier,
        } if tool_name == "exit_plan_mode"
            && qualifier
                .as_deref()
                .map(|value| value == "plan_review")
                .unwrap_or(true)
    )
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

pub fn operation_diff_summary(i18n: &I18n, line_count: usize) -> String {
    i18n.text_args(
        "operation-diff-summary",
        &fl_args!("count" => line_count as i64),
    )
}

pub fn operation_nested_task_summary(i18n: &I18n, title: &str, status: ExecutionStatus) -> String {
    i18n.text_args(
        "operation-nested-task-summary",
        &fl_args!(
            "title" => title,
            "status" => execution_status_label(i18n, status),
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

pub fn message_tool_summary(i18n: &I18n, status: ExecutionStatus, label: &str) -> String {
    i18n.text_args(
        "message-tool-summary",
        &fl_args!(
            "status" => execution_status_label(i18n, status),
            "label" => label,
        ),
    )
}

pub fn worktree_picker_title(i18n: &I18n) -> String {
    t(i18n, "overlay-worktree-title")
}

pub fn worktree_picker_prompt(i18n: &I18n) -> String {
    t(i18n, "command-worktree-summary")
}

pub fn worktree_ready_message(i18n: &I18n, path: &str, branch: Option<&str>) -> String {
    let branch = branch
        .map(str::to_owned)
        .unwrap_or_else(|| t(i18n, "value-unknown"));
    i18n.text_args(
        "flash-worktree-ready",
        &fl_args!("path" => path, "branch" => branch),
    )
}

pub fn worktree_attached_message(i18n: &I18n, path: &str, branch: Option<&str>) -> String {
    let branch = branch
        .map(str::to_owned)
        .unwrap_or_else(|| t(i18n, "value-unknown"));
    i18n.text_args(
        "flash-worktree-attached",
        &fl_args!("path" => path, "branch" => branch),
    )
}

pub fn worktree_exit_message(i18n: &I18n, action: Option<&str>, path: &str) -> String {
    i18n.text_args(
        "flash-worktree-exited",
        &fl_args!(
            "action" => worktree_action_label(i18n, action),
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

fn worktree_action_label(i18n: &I18n, action: Option<&str>) -> String {
    match action.unwrap_or_default().to_ascii_lowercase().as_str() {
        "" => t(i18n, "worktree-action-exit"),
        "keep" => t(i18n, "worktree-action-keep"),
        "remove" => t(i18n, "worktree-action-remove"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize_fluent_markup(text: String) -> String {
        text.replace(['\u{2068}', '\u{2069}'], "")
    }

    #[test]
    fn worktree_and_command_feedback_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);

        assert_eq!(worktree_picker_title(&i18n), "Worktree");
        assert_eq!(
            worktree_picker_prompt(&i18n),
            "查看当前活跃与托管的 worktree"
        );
        assert_eq!(
            normalize_fluent_markup(worktree_ready_message(&i18n, "/tmp/wt", Some("main"))),
            "worktree 已就绪：/tmp/wt (main)"
        );
        assert_eq!(
            normalize_fluent_markup(worktree_exit_message(&i18n, Some("remove"), "/tmp/wt")),
            "worktree 已移除：/tmp/wt"
        );
        assert_eq!(
            normalize_fluent_markup(commit_created_message(&i18n, "abc123", "feat: update")),
            "已创建 commit：abc123 feat: update"
        );
        assert_eq!(
            normalize_fluent_markup(pull_request_created_message(
                &i18n,
                "https://example.test/pr/1",
            )),
            "已创建 pull request：https://example.test/pr/1"
        );
    }

    #[test]
    fn transcript_and_composer_messages_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);

        assert_eq!(
            no_session_selected_text(&i18n),
            "尚未选择会话。\n按 Alt+S 选择会话，或直接在输入框中开始输入以创建新会话。"
        );
        assert_eq!(
            normalize_fluent_markup(transcript_export_path_is_directory_error(
                &i18n,
                Path::new("/tmp/export"),
            )),
            "导出路径是一个目录：/tmp/export"
        );
        assert_eq!(
            normalize_fluent_markup(attachment_inspect_failed_message(
                &i18n,
                Path::new("/tmp/image.png"),
                "boom",
            )),
            "检查附件 /tmp/image.png 失败：boom"
        );
        assert_eq!(
            normalize_fluent_markup(composer_drafts_save_failed_message(&i18n, "disk full")),
            "保存 composer drafts 失败：disk full"
        );
        assert_eq!(
            normalize_fluent_markup(prompt_history_save_failed_message(&i18n, "disk full")),
            "保存 prompt history 失败：disk full"
        );
        assert_eq!(
            composer_placeholder_range_invalid_error(&i18n),
            "composer 占位符范围无效"
        );
        assert_eq!(
            composer_placeholder_out_of_sync_error(&i18n),
            "composer 占位符状态不同步"
        );
        assert_eq!(
            normalize_fluent_markup(composer_missing_staged_item_error(&i18n, "[image foo.png]",)),
            "缺少对应的暂存项目：[image foo.png]"
        );
    }
}
