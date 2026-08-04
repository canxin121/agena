//! Presentation state and input reducer for an interactive permission prompt.
//!
//! The application owns the permission request, Domain reply values, rule
//! editor, and Runtime submission. This module owns only the prompt's pages,
//! selected choice, and keyboard/navigation policy.

use crossterm::event::KeyEvent;

use agena_tui_components::{
    ParagraphSection, SelectionCursor, StackedDialogSection, StackedDialogSectionHeight,
    StackedDialogSpec, SurfaceMode, render_stacked_dialog,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Borders,
};

use crate::i18n::I18n;
use crate::keymap::{KeyAction, KeyContext, resolve};
use tui_markdown::from_str as markdown_to_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPromptDecision {
    Allow,
    Deny,
}

/// Tone selected by the App's one-way Domain-to-display projection. The TUI
/// translates it into terminal styling without receiving a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPromptLineTone {
    Normal,
    Muted,
    Strong,
}

/// A terminal-safe display row for the permission prompt. It deliberately
/// carries no Domain action, reply, policy, or persistence value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPromptLine {
    pub text: String,
    pub tone: PermissionPromptLineTone,
    /// Optional Markdown source rendered in place of `text`. Code fences,
    /// inline code, and headings get terminal styling (monospace / bold).
    pub markdown: Option<String>,
}

impl PermissionPromptLine {
    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: PermissionPromptLineTone::Normal,
            markdown: None,
        }
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: PermissionPromptLineTone::Muted,
            markdown: None,
        }
    }

    pub fn strong(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: PermissionPromptLineTone::Strong,
            markdown: None,
        }
    }

    /// A row rendered from Markdown source (e.g. a shell command in a code
    /// fence). The tone only applies when `markdown` is absent.
    pub fn markdown(markdown: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            tone: PermissionPromptLineTone::Normal,
            markdown: Some(markdown.into()),
        }
    }
}

/// Opaque read-only content supplied by the App once when it receives a
/// Domain permission request. The TUI owns page selection and rendering.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionPromptContent {
    pub overview: Vec<PermissionPromptLine>,
    pub details: Vec<PermissionPromptLine>,
}

/// Live async state for the "auto-approve" choice. `PermissionPromptContent`
/// stays immutable; this state is rendered as an overlay on the choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionPromptAutoApproveStatus {
    /// The classifier request is in flight; the choice is disabled.
    Requesting,
    /// The classifier request failed; `reason` is shown below the choices.
    Failed(String),
}

pub fn decision_label(i18n: &I18n, decision: PermissionPromptDecision) -> String {
    i18n.text(match decision {
        PermissionPromptDecision::Allow => "value-allow",
        PermissionPromptDecision::Deny => "value-deny",
    })
}

pub fn choice_decision_label(i18n: &I18n, decision: PermissionPromptDecision) -> String {
    i18n.text(match decision {
        PermissionPromptDecision::Allow => "overlay-permission-choice-allow",
        PermissionPromptDecision::Deny => "overlay-permission-choice-deny",
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPromptDetailsReturn {
    Action,
    Scope(PermissionPromptDecision),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPromptPage {
    Action,
    Scope(PermissionPromptDecision),
    Details(PermissionPromptDetailsReturn),
}

impl PermissionPromptPage {
    pub fn choice_count(self) -> usize {
        match self {
            Self::Action => 5,
            Self::Scope(_) => 5,
            Self::Details(_) => 0,
        }
    }

    pub fn is_details(self) -> bool {
        matches!(self, Self::Details(_))
    }
}

pub fn title(i18n: &I18n, page: PermissionPromptPage) -> String {
    let base = i18n.text("overlay-permission-title");
    match page {
        PermissionPromptPage::Action => base,
        PermissionPromptPage::Scope(decision) => {
            format!("{base} · {}", decision_label(i18n, decision))
        }
        PermissionPromptPage::Details(_) => {
            format!("{base} · {}", i18n.text("overlay-permission-details-title"))
        }
    }
}

pub fn footer(i18n: &I18n, page: PermissionPromptPage) -> String {
    i18n.text(match page {
        PermissionPromptPage::Action => "overlay-permission-footer-action",
        PermissionPromptPage::Scope(_) => "overlay-permission-footer-scope",
        PermissionPromptPage::Details(_) => "overlay-permission-footer-details",
    })
}

#[derive(Debug, Clone)]
pub struct PermissionPromptPresentation {
    content: PermissionPromptContent,
    page: PermissionPromptPage,
    selection: SelectionCursor,
}

impl PermissionPromptPresentation {
    pub fn new(content: PermissionPromptContent) -> Self {
        Self {
            content,
            page: PermissionPromptPage::Action,
            selection: SelectionCursor::default(),
        }
    }

    pub fn content(&self) -> &PermissionPromptContent {
        &self.content
    }

    pub fn active_content(&self) -> &[PermissionPromptLine] {
        if self.page.is_details() {
            self.content.details.as_slice()
        } else {
            self.content.overview.as_slice()
        }
    }

    pub fn page(&self) -> PermissionPromptPage {
        self.page
    }

    pub fn selected(&self) -> usize {
        self.selection.selected
    }

    pub fn open_scope(&mut self, decision: PermissionPromptDecision) {
        self.page = PermissionPromptPage::Scope(decision);
        self.selection.selected = 0;
    }

    pub fn open_details(&mut self) -> bool {
        self.page = match self.page {
            PermissionPromptPage::Action => {
                PermissionPromptPage::Details(PermissionPromptDetailsReturn::Action)
            }
            PermissionPromptPage::Scope(decision) => {
                PermissionPromptPage::Details(PermissionPromptDetailsReturn::Scope(decision))
            }
            PermissionPromptPage::Details(_) => return false,
        };
        self.selection.selected = 0;
        true
    }

    fn back(&mut self) -> PermissionPromptEffect {
        match self.page {
            PermissionPromptPage::Action => PermissionPromptEffect::Close,
            PermissionPromptPage::Scope(_) => {
                self.page = PermissionPromptPage::Action;
                self.selection.selected = 0;
                PermissionPromptEffect::KeepOpen
            }
            PermissionPromptPage::Details(return_to) => {
                self.page = match return_to {
                    PermissionPromptDetailsReturn::Action => PermissionPromptPage::Action,
                    PermissionPromptDetailsReturn::Scope(decision) => {
                        PermissionPromptPage::Scope(decision)
                    }
                };
                PermissionPromptEffect::KeepOpen
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPromptEffect {
    KeepOpen,
    Close,
    Activate {
        page: PermissionPromptPage,
        selected: usize,
    },
}

pub fn handle_key(
    presentation: &mut PermissionPromptPresentation,
    key: KeyEvent,
) -> PermissionPromptEffect {
    match resolve(KeyContext::PermissionPrompt, key) {
        Some(KeyAction::Back) => presentation.back(),
        Some(KeyAction::MoveUp) if !presentation.page.is_details() => {
            presentation
                .selection
                .move_by(presentation.page.choice_count(), -1);
            PermissionPromptEffect::KeepOpen
        }
        Some(KeyAction::MoveDown) if !presentation.page.is_details() => {
            presentation
                .selection
                .move_by(presentation.page.choice_count(), 1);
            PermissionPromptEffect::KeepOpen
        }
        Some(KeyAction::Activate) if !presentation.page.is_details() => {
            PermissionPromptEffect::Activate {
                page: presentation.page,
                selected: presentation.selection.selected,
            }
        }
        _ => PermissionPromptEffect::KeepOpen,
    }
}

/// Renders the entire permission prompt from opaque display-only content.
/// The App retains the Domain request, reply validation, rule-editor routing,
/// and Runtime submission effect. `auto_approve` carries the live async status
/// of the "auto-approve" choice (in-flight or failed).
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &PermissionPromptPresentation,
    auto_approve: Option<&PermissionPromptAutoApproveStatus>,
    i18n: &I18n,
) {
    let body = Text::from(content_lines(presentation.active_content()));
    let page = presentation.page();
    let footer = Text::from(footer(i18n, page));
    let mut sections = vec![StackedDialogSection::Paragraph(ParagraphSection {
        height: StackedDialogSectionHeight::AutoText { min: 6, max: 40 },
        title: None,
        borders: Borders::NONE,
        body,
        wrap: true,
        scroll: None,
        alignment: None,
    })];
    if !page.is_details() {
        sections.push(StackedDialogSection::Paragraph(ParagraphSection {
            height: StackedDialogSectionHeight::AutoText { min: 3, max: 6 },
            title: None,
            borders: Borders::NONE,
            body: Text::from(choice_lines(presentation, auto_approve, i18n)),
            wrap: true,
            scroll: None,
            alignment: None,
        }));
    }
    sections.push(StackedDialogSection::Paragraph(ParagraphSection {
        height: StackedDialogSectionHeight::AutoText { min: 1, max: 2 },
        title: None,
        borders: Borders::NONE,
        body: footer,
        wrap: true,
        scroll: None,
        alignment: None,
    }));
    render_stacked_dialog(
        frame,
        area,
        SurfaceMode::Overlay,
        &StackedDialogSpec {
            title: title(i18n, page).into(),
            target_width: 108,
            sections,
        },
    );
}

fn content_lines(content: &[PermissionPromptLine]) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(content.len());
    for line in content {
        if let Some(markdown) = line.markdown.as_deref() {
            let markdown = markdown.trim();
            if markdown.is_empty() {
                lines.push(Line::from(""));
                continue;
            }
            let rendered = markdown_to_text(markdown);
            lines.extend(rendered.lines.into_iter().map(|md_line| {
                Line::from(
                    md_line
                        .spans
                        .into_iter()
                        .map(|span| Span::styled(sanitize_display_text(&span.content), span.style))
                        .collect::<Vec<_>>(),
                )
            }));
            continue;
        }
        lines.push(Line::from(Span::styled(
            sanitize_display_text(line.text.as_str()),
            match line.tone {
                PermissionPromptLineTone::Normal => Style::default(),
                PermissionPromptLineTone::Muted => {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                }
                PermissionPromptLineTone::Strong => Style::default().add_modifier(Modifier::BOLD),
            },
        )));
    }
    lines
}

fn choice_lines(
    presentation: &PermissionPromptPresentation,
    auto_approve: Option<&PermissionPromptAutoApproveStatus>,
    i18n: &I18n,
) -> Vec<Line<'static>> {
    let page = presentation.page();
    let heading = match page {
        PermissionPromptPage::Action => "overlay-permission-decision-heading",
        PermissionPromptPage::Scope(_) => "overlay-permission-scope-heading",
        PermissionPromptPage::Details(_) => return Vec::new(),
    };
    let mut lines = vec![Line::from(Span::styled(
        sanitize_display_text(i18n.text(heading).as_str()),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for (index, label) in choice_labels(i18n, page, auto_approve)
        .into_iter()
        .enumerate()
    {
        let selected = index == presentation.selected();
        let style = if selected {
            agena_tui_components::theme::selection_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}", if selected { ">> " } else { "   " }, label),
            style,
        )));
    }
    if let Some(PermissionPromptAutoApproveStatus::Failed(reason)) = auto_approve {
        let reason = sanitize_display_text(reason.as_str());
        lines.push(Line::from(Span::styled(
            reason,
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
    }
    lines
}

fn choice_labels(
    i18n: &I18n,
    page: PermissionPromptPage,
    auto_approve: Option<&PermissionPromptAutoApproveStatus>,
) -> Vec<String> {
    match page {
        PermissionPromptPage::Action => {
            let mut labels = vec![
                choice_decision_label(i18n, PermissionPromptDecision::Allow),
                choice_decision_label(i18n, PermissionPromptDecision::Deny),
                i18n.text("overlay-permission-choice-edit-rule"),
            ];
            let auto_approve_label = match auto_approve {
                Some(PermissionPromptAutoApproveStatus::Requesting) => {
                    i18n.text("overlay-permission-choice-auto-approve-busy")
                }
                _ => i18n.text("overlay-permission-choice-auto-approve"),
            };
            labels.push(auto_approve_label);
            labels.push(i18n.text("overlay-permission-details-title"));
            labels
        }
        PermissionPromptPage::Scope(_) => vec![
            i18n.text("overlay-permission-choice-once"),
            i18n.text("overlay-permission-choice-session"),
            i18n.text("overlay-permission-choice-workspace"),
            i18n.text("overlay-permission-choice-global"),
            i18n.text("overlay-permission-details-title"),
        ],
        PermissionPromptPage::Details(_) => Vec::new(),
    }
}

fn sanitize_display_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        match ch {
            '\r' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            '\n' | '\t' => out.push(ch),
            value if value.is_control() => out.push(' '),
            value => out.push(value),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        PermissionPromptContent, PermissionPromptDecision, PermissionPromptEffect,
        PermissionPromptLine, PermissionPromptPage, PermissionPromptPresentation, handle_key,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn back_from_scope_returns_to_action_and_resets_selection() {
        let mut presentation =
            PermissionPromptPresentation::new(PermissionPromptContent::default());
        presentation.open_scope(PermissionPromptDecision::Allow);
        let _ = handle_key(
            &mut presentation,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        );

        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            ),
            PermissionPromptEffect::KeepOpen
        );
        assert_eq!(presentation.page(), PermissionPromptPage::Action);
        assert_eq!(presentation.selected(), 0);
    }

    #[test]
    fn activation_exposes_only_presentation_page_and_index() {
        let mut presentation =
            PermissionPromptPresentation::new(PermissionPromptContent::default());

        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            ),
            PermissionPromptEffect::Activate {
                page: PermissionPromptPage::Action,
                selected: 0,
            }
        );
    }

    #[test]
    fn details_page_switches_to_opaque_detail_content() {
        let mut presentation = PermissionPromptPresentation::new(PermissionPromptContent {
            overview: vec![PermissionPromptLine::normal("overview")],
            details: vec![PermissionPromptLine::muted("details")],
        });

        assert_eq!(presentation.active_content()[0].text, "overview");
        assert!(presentation.open_details());
        assert_eq!(presentation.active_content()[0].text, "details");
    }
}
