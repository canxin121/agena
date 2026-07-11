use super::super::{
    I18n, MessagePart, Modifier, PartContent, RenderedLine, RequestPart, Style,
    file_change_list_item_text, file_change_style, first_non_empty_preview_line, push_label_value,
    push_multiline, push_section_heading, tool_execution_preview, transcript_part_content, ui_text,
};

pub(in crate::app) fn render_file_changes(
    changes: &[agena::message::FileChangeRecord],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    if changes.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-file-changes")),
        Style::default()
            .fg(agena_tui_components::theme::special_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    for entry in changes {
        push_label_value(
            out,
            "      - ",
            &file_change_list_item_text(entry, i18n),
            file_change_style(entry.kind),
            width,
        );
    }
}

pub(in crate::app) fn render_checklist(
    items: &[agena::message::TodoItem],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    if items.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-todo-list")),
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    for item in items {
        push_label_value(
            out,
            "      - ",
            &format!(
                "[{}|{}] {}",
                ui_text::todo_status_label(i18n, item.status),
                ui_text::todo_priority_label(i18n, item.priority),
                item.content
            ),
            Style::default(),
            width,
        );
    }
}

pub(in crate::app) fn render_permission_request(
    permission: &agena::message::InteractiveRequestPart<
        agena::permission::PermissionRequest,
        agena::permission::PermissionReply,
    >,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    push_section_heading(
        out,
        &format!("  {}", ui_text::message_permission_heading(i18n)),
        Style::default()
            .fg(agena_tui_components::theme::special_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    push_multiline(
        out,
        "    ",
        ui_text::permission_summary(i18n, permission).as_str(),
        Style::default().fg(agena_tui_components::theme::special_color()),
        width,
    );
}

pub(in crate::app) fn render_user_input_request(
    request: &agena::message::InteractiveRequestPart<
        agena::message::UserInputRequest,
        agena::message::UserInputReply,
    >,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    push_multiline(
        out,
        "  ",
        &i18n.text_args(
            "message-awaiting-user-input",
            &crate::fl_args!("request_id" => request.request.request_id.as_str()),
        ),
        Style::default().fg(agena_tui_components::theme::special_color()),
        width,
    );
    for question in &request.request.questions {
        push_multiline(
            out,
            "    ",
            &ui_text::message_question_line(i18n, question.question.as_str(), question.id.as_str()),
            Style::default(),
            width,
        );
    }
}

pub(in crate::app) fn preview_for_part(part: &MessagePart, i18n: &I18n) -> Option<String> {
    match transcript_part_content(part) {
        PartContent::Text(text) => first_non_empty_preview_line(text.text.as_str()),
        PartContent::Reasoning(reasoning) => {
            let summary = reasoning.preferred_text();
            first_non_empty_preview_line(summary.as_str())
        }
        PartContent::Operation(tool) => Some(tool_execution_preview(part, tool, i18n)),
        PartContent::Error(error) => Some(ui_text::message_error_text(
            i18n,
            error.code.as_str(),
            error.message.as_str(),
        )),
        PartContent::Attachment(attachment) => attachment.attachments.first().map(|item| {
            item.title
                .as_ref()
                .or(item.filename.as_ref())
                .cloned()
                .unwrap_or_else(|| item.mime.clone())
        }),
        PartContent::Request(RequestPart::Permission(permission)) => {
            Some(ui_text::permission_summary(i18n, permission))
        }
        PartContent::Request(RequestPart::UserInput(request)) => request
            .request
            .questions
            .first()
            .map(|question| question.question.clone()),
    }
}
