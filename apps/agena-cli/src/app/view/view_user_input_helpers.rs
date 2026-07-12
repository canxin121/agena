pub(in crate::app) fn user_input_nav_line(
    i18n: &I18n,
    dialog: &UserInputOverlay,
    answered_color: Color,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, question) in dialog.request.questions.iter().enumerate() {
        let answered = dialog
            .answers
            .get(&question.id)
            .map(|draft| !user_input_answer_values(question, draft).is_empty())
            .unwrap_or(false);
        let label = if question.header.trim().is_empty() {
            format!("Q{}", index + 1)
        } else {
            question.header.clone()
        };
        let text = format!(
            " {} {} ",
            if answered { "[x]" } else { "[ ]" },
            truncate_display_text(sanitize_display_text(label.as_str()).as_str(), 12)
        );
        let selected = dialog.state.selected_question() == index
            && dialog.state.screen() == QuestionFlowScreen::Question;
        let style = if selected {
            selection_highlight_style()
        } else if answered {
            Style::default()
                .fg(answered_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    if !App::user_input_review_hidden(dialog) {
        spans.push(Span::styled(
            format!(" [>] {} ", user_input_submit_label(i18n, &dialog.request)),
            if dialog.state.screen() == QuestionFlowScreen::Review {
                selection_highlight_style()
            } else {
                Style::default()
            },
        ));
    }
    Line::from(spans)
}

pub(in crate::app) fn user_input_overlay_title(i18n: &I18n, request: &UserInputRequest) -> String {
    let title = request.title.trim();
    if title.is_empty() {
        ui_text::t(i18n, "overlay-user-input-title")
    } else {
        sanitize_display_text(title)
    }
}

pub(in crate::app) fn user_input_review_question(
    request: &UserInputRequest,
) -> Option<&UserInputQuestion> {
    let question = request.questions.first()?;
    if request.kind.trim() != "review" || request.questions.len() != 1 || question.multiple {
        return None;
    }
    (!question.options.is_empty()).then_some(question)
}

pub(in crate::app) fn user_input_request_is_review(request: &UserInputRequest) -> bool {
    user_input_review_question(request).is_some()
}

pub(in crate::app) fn user_input_submit_label(i18n: &I18n, request: &UserInputRequest) -> String {
    let label = request.submit_label.trim();
    if label.is_empty() {
        ui_text::t(i18n, "overlay-user-input-submit")
    } else {
        sanitize_display_text(label)
    }
}

pub(in crate::app) fn user_input_footer_text(
    i18n: &I18n,
    request: &UserInputRequest,
    key: &str,
) -> String {
    let mut footer = ui_text::t(i18n, key);
    let cancel = request.cancel_label.trim();
    if !cancel.is_empty() {
        footer.push_str(" · Esc ");
        footer.push_str(sanitize_display_text(cancel).as_str());
    }
    footer
}

pub(in crate::app) fn review_request_body_markdown(body_markdown: &str) -> Text<'static> {
    user_input_markdown_text(body_markdown, None)
}

pub(in crate::app) fn user_input_markdown_text(
    body_markdown: &str,
    style: Option<Style>,
) -> Text<'static> {
    let markdown = body_markdown.trim();
    if markdown.is_empty() {
        return Text::from(vec![Line::from("")]);
    }
    let rendered = markdown_to_text(markdown);
    Text::from(
        rendered
            .lines
            .into_iter()
            .map(|line| {
                let spans = line
                    .spans
                    .into_iter()
                    .map(|span| {
                        Span::styled(
                            sanitize_display_text(span.content),
                            style.map_or(span.style, |base| span.style.patch(base)),
                        )
                    })
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect::<Vec<_>>(),
    )
}

pub(in crate::app) fn user_input_body_markdown_lines(
    body_markdown: &str,
    style: Option<Style>,
) -> Vec<Line<'static>> {
    user_input_markdown_text(body_markdown, style).lines
}

pub(in crate::app) fn user_input_timeout_line(
    i18n: &I18n,
    request: &UserInputRequest,
) -> Option<Line<'static>> {
    let text = user_input_timeout_text(i18n, request)?;
    Some(Line::from(vec![
        Span::styled(
            "◷ ",
            Style::default().fg(agena_tui_components::theme::warning_color()),
        ),
        Span::styled(
            text,
            Style::default().fg(agena_tui_components::theme::warning_color()),
        ),
    ]))
}

pub(in crate::app) fn user_input_timeout_text(
    i18n: &I18n,
    request: &UserInputRequest,
) -> Option<String> {
    let timeout_ms = request.auto_resolution_ms?;
    let deadline = request.created_at + chrono::Duration::milliseconds(timeout_ms as i64);
    let remaining_ms = deadline
        .signed_duration_since(chrono::Utc::now())
        .num_milliseconds()
        .max(0) as u64;
    let remaining_secs = remaining_ms.div_ceil(1000);
    let remaining = format!("{}:{:02}", remaining_secs / 60, remaining_secs % 60);
    Some(sanitize_display_text(i18n.text_args(
        "overlay-user-input-auto-resolve",
        &crate::fl_args!("remaining" => remaining),
    )))
}

pub(in crate::app) fn user_input_answer_summary(
    i18n: &I18n,
    question: &UserInputQuestion,
    draft: &UserInputAnswerDraft,
) -> String {
    let values = user_input_answer_values(question, draft);
    if values.is_empty() {
        ui_text::t(i18n, "overlay-user-input-unanswered")
    } else {
        values.join(", ")
    }
}

pub(in crate::app) fn highlight_search_line(
    text: &str,
    base_style: Style,
    query: &str,
    active_match: bool,
    has_match: bool,
) -> Line<'static> {
    let line_style = if active_match {
        base_style.patch(agena_tui_components::theme::selection_style())
    } else if has_match {
        base_style
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::UNDERLINED)
    } else {
        base_style
    };

    if !has_match || query.trim().is_empty() {
        return Line::from(Span::styled(text.to_string(), line_style));
    }

    let ranges = find_search_ranges(text, query);
    if ranges.is_empty() {
        return Line::from(Span::styled(text.to_string(), line_style));
    }

    let mut spans = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if cursor < range.start {
            spans.push(Span::styled(
                text[cursor..range.start].to_string(),
                line_style,
            ));
        }
        let match_style = if active_match {
            line_style
        } else {
            line_style
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::UNDERLINED)
        };
        spans.push(Span::styled(text[range.clone()].to_string(), match_style));
        cursor = range.end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), line_style));
    }

    Line::from(spans)
}
use super::{
    App, Color, I18n, Line, Modifier, QuestionFlowScreen, Span, Style, Text, UserInputAnswerDraft,
    UserInputOverlay, UserInputQuestion, UserInputRequest, find_search_ranges, markdown_to_text,
    sanitize_display_text, selection_highlight_style, truncate_display_text, ui_text,
    user_input_answer_values,
};
