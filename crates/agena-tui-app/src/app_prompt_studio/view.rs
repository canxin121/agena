use std::cell::Cell;

use agena_prompt_layers::{
    BuiltinPrompt, BuiltinPromptWhen, FragmentId, LayerMode, PromptFragment, PromptScope,
};
use agena_tui::i18n::I18n;
use agena_tui_components::{
    SurfaceMode, WorkbenchFrameSpec, pointer, render_workbench_frame, theme,
    truncate_display_text_with_suffix,
};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};

use super::{prompt_library, prompt_studio_entries, prompt_title, saved_prompt_choices};
use crate::{
    PromptStudioOverlay, PromptStudioPickerAction, PromptStudioScope, sanitize_terminal_text,
};

pub(crate) struct PromptRow {
    pub title: String,
    pub description: String,
    pub excerpt: String,
    pub body: String,
    pub position: Option<usize>,
    pub builtin: bool,
}

impl PromptRow {
    pub(crate) fn builtin(prompt: &BuiltinPrompt, i18n: &I18n, read_body: bool) -> Self {
        let suffix = prompt.id.as_str().trim_start_matches("agena.system.");
        let title = i18n
            .try_text(&format!("prompt-builtin-{suffix}"))
            .unwrap_or_else(|| prompt.title.clone());
        let description = i18n.text(match prompt.when {
            BuiltinPromptWhen::Always => "prompt-studio-every-session",
            BuiltinPromptWhen::UserFacing => "prompt-studio-user-facing",
            BuiltinPromptWhen::Subagent => "prompt-studio-delegated",
            BuiltinPromptWhen::Tools => "prompt-studio-tools",
            BuiltinPromptWhen::UserFacingTools => "prompt-studio-user-facing-tools",
        });
        Self {
            title,
            description,
            excerpt: prompt_excerpt(&prompt.body, false),
            body: if read_body {
                sanitize_terminal_text(&prompt.body)
            } else {
                String::new()
            },
            position: None,
            builtin: true,
        }
    }

    pub(crate) fn custom<'a>(
        prompt: &'a PromptFragment,
        i18n: &I18n,
        read_body: bool,
        lookup: impl Fn(&FragmentId) -> Option<&'a PromptFragment>,
    ) -> Self {
        let description = if prompt.id.is_reusable_builtin() {
            i18n.text("prompt-studio-reusable")
        } else if prompt.category.is_empty() {
            i18n.text(if prompt.parts.is_empty() {
                "prompt-studio-custom"
            } else {
                "prompt-studio-composition"
            })
        } else {
            sanitize_terminal_text(&prompt.category)
        };
        let body = if !read_body {
            String::new()
        } else if prompt.parts.is_empty() {
            sanitize_terminal_text(&prompt.body)
        } else {
            prompt
                .parts
                .iter()
                .enumerate()
                .map(|(index, part)| format!("{}. {part}", index + 1))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Self {
            title: reusable_title(&prompt.id, i18n)
                .unwrap_or_else(|| sanitize_terminal_text(prompt_title(prompt))),
            description,
            excerpt: if prompt.parts.is_empty() {
                prompt_excerpt(&prompt.body, false)
            } else {
                composition_excerpt(prompt, lookup)
            },
            body,
            position: None,
            builtin: false,
        }
    }
}

fn reusable_title(id: &FragmentId, i18n: &I18n) -> Option<String> {
    let suffix = id.as_str().strip_prefix("agena.profile.")?;
    i18n.try_text(&format!("prompt-builtin-{suffix}"))
}

pub(crate) fn prompt_excerpt(body: &str, has_more: bool) -> String {
    let mut lines = body
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return String::new();
    };
    let marker = first.split_whitespace().next().unwrap_or_default();
    let line = if marker.len() <= 6 && marker.bytes().all(|byte| byte == b'#') {
        lines.next().unwrap_or(first)
    } else {
        first
    };
    let mut text = sanitize_terminal_text(line)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !text.is_empty() && (has_more || lines.next().is_some()) && !text.ends_with('…') {
        text.push('…');
    }
    truncate_display_text_with_suffix(&text, 120, "…")
}

fn composition_excerpt<'a>(
    prompt: &'a PromptFragment,
    lookup: impl Fn(&FragmentId) -> Option<&'a PromptFragment>,
) -> String {
    fn visit<'a>(
        prompt: &'a PromptFragment,
        lookup: &impl Fn(&FragmentId) -> Option<&'a PromptFragment>,
        depth: usize,
        remaining: &mut usize,
        bodies: &mut Vec<&'a str>,
    ) -> Option<()> {
        if depth >= 32 || *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        if prompt.parts.is_empty() && bodies.len() < 2 && !prompt.body.trim().is_empty() {
            bodies.push(&prompt.body);
        }
        for id in &prompt.parts {
            visit(lookup(id)?, lookup, depth + 1, remaining, bodies)?;
        }
        Some(())
    }

    let mut bodies = Vec::with_capacity(2);
    let mut remaining = 256;
    if visit(prompt, &lookup, 0, &mut remaining, &mut bodies).is_none() {
        return String::new();
    }
    prompt_excerpt(
        bodies.first().copied().unwrap_or_default(),
        bodies.len() > 1,
    )
}

fn prompt_rows(dialog: &PromptStudioOverlay, i18n: &I18n, selected: usize) -> Vec<PromptRow> {
    if let Some(preview) = &dialog.preview {
        return preview
            .fragments
            .iter()
            .enumerate()
            .map(|(index, fragment)| {
                let builtin = dialog
                    .builtins
                    .iter()
                    .find(|prompt| prompt.id == fragment.id);
                let mut row = builtin.map_or_else(
                    || PromptRow {
                        title: if fragment.scope == PromptScope::Runtime
                            && fragment.id.as_str() == "agena.runtime.session-relation"
                        {
                            i18n.text("prompt-studio-session-relation")
                        } else {
                            reusable_title(&fragment.id, i18n)
                                .unwrap_or_else(|| clean_title(&fragment.title, &fragment.id))
                        },
                        description: i18n.text(match fragment.scope {
                            PromptScope::Global => "prompt-studio-global",
                            PromptScope::Workspace => "prompt-studio-workspace",
                            PromptScope::Model => "prompt-studio-model",
                            PromptScope::Profile => "settings-agent-profile-label",
                            PromptScope::Session => "prompt-studio-session",
                            PromptScope::Bot => "bot-manager-title",
                            _ => "prompt-studio-runtime",
                        }),
                        excerpt: String::new(),
                        body: String::new(),
                        position: None,
                        builtin: false,
                    },
                    |prompt| PromptRow::builtin(prompt, i18n, false),
                );
                row.position = Some(index + 1);
                row.excerpt = prompt_excerpt(&fragment.text, false);
                if index == selected {
                    row.body = sanitize_terminal_text(&fragment.text);
                }
                row
            })
            .collect();
    }
    if dialog.preview_request.is_some() {
        return Vec::new();
    }
    let library = prompt_library(dialog);
    if let Some(action) = dialog.picker {
        let mut rows: Vec<_> = if action == PromptStudioPickerAction::Browse {
            dialog
                .builtins
                .iter()
                .enumerate()
                .map(|(index, prompt)| PromptRow::builtin(prompt, i18n, index == selected))
                .collect()
        } else {
            Vec::new()
        };
        let count = rows.len();
        rows.extend(
            saved_prompt_choices(dialog, action)
                .into_iter()
                .enumerate()
                .map(|(index, prompt)| {
                    PromptRow::custom(&prompt, i18n, index + count == selected, |id| {
                        library.get(id).copied()
                    })
                }),
        );
        return rows;
    }
    prompt_studio_entries(dialog)
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut row =
                if let Some(prompt) = dialog.builtins.iter().find(|prompt| prompt.id == entry.id) {
                    PromptRow::builtin(prompt, i18n, index == selected)
                } else if let Some(prompt) = library.get(&entry.id) {
                    PromptRow::custom(prompt, i18n, index == selected, |id| {
                        library.get(id).copied()
                    })
                } else {
                    PromptRow {
                        title: entry.id.to_string(),
                        description: i18n.text("prompt-studio-external"),
                        excerpt: String::new(),
                        body: i18n.text("prompt-studio-external-detail"),
                        position: None,
                        builtin: false,
                    }
                };
            row.position = entry.order_index.map(|index| index + 1);
            if row.position.is_none() {
                row.description = i18n.text("prompt-studio-unselected");
            }
            row
        })
        .collect()
}

fn clean_title(title: &str, id: &FragmentId) -> String {
    if title.trim().is_empty() {
        id.to_string()
    } else {
        sanitize_terminal_text(title)
    }
}

pub(super) fn render(frame: &mut Frame, area: Rect, dialog: &PromptStudioOverlay, i18n: &I18n) {
    let in_preview = dialog.preview.is_some() || dialog.preview_request.is_some();
    let (heading, keys, selected) = if in_preview {
        (
            "prompt-studio-preview",
            "prompt-studio-browse-keys",
            dialog.preview_selected,
        )
    } else if let Some(action) = dialog.picker {
        let (title, keys) = match action {
            PromptStudioPickerAction::Browse => {
                ("prompt-studio-library", "prompt-studio-browse-keys")
            }
            PromptStudioPickerAction::Order => ("prompt-studio-add", "prompt-studio-pick-keys"),
            PromptStudioPickerAction::Hide => ("prompt-studio-hide", "prompt-studio-pick-keys"),
            PromptStudioPickerAction::Unhide => ("prompt-studio-unhide", "prompt-studio-pick-keys"),
        };
        (title, keys, dialog.picker_selected)
    } else {
        (
            "prompt-studio-order",
            "prompt-studio-order-keys",
            dialog.selected,
        )
    };
    let rows = prompt_rows(dialog, i18n, selected);
    let row = rows.get(selected);
    let footer = i18n.text(if dialog.detail_open {
        "prompt-studio-read-keys"
    } else {
        keys
    });
    let title = if dialog.picker.is_some() || in_preview {
        heading
    } else {
        "prompt-studio-title"
    };
    let spec = WorkbenchFrameSpec::new(i18n.text(title).into(), footer.into(), 112, 27)
        .footer_max_height(if area.height >= 18 { 4 } else { 2 });
    let workbench = render_workbench_frame(frame, area, SurfaceMode::Overlay, &spec);
    let body = workbench.body;
    if body.height < 2 || body.width < 4 {
        return;
    }
    if dialog.detail_open
        && let Some(row) = row
    {
        render_detail(
            frame,
            body.inner(Margin::new(1, 0)),
            row,
            Some(&dialog.detail_scroll),
            i18n,
        );
        return;
    }
    let context = if let Some(preview) = &dialog.preview {
        let mut context = format!(
            "{}  ·  {} B",
            i18n.text("prompt-studio-saved-session"),
            preview.total_bytes
        );
        if preview.over_budget || !preview.warnings.is_empty() {
            context.push_str(&format!(
                "  ·  {} ({})",
                i18n.text("prompt-studio-warnings"),
                preview.warnings.len()
            ));
        }
        Line::styled(context, theme::muted_style())
    } else {
        scope_line(dialog, i18n, body.width)
    };
    let status = if dialog.status.is_empty() && !in_preview && dialog.picker.is_none() {
        let mut parts = Vec::new();
        if !dialog.draft().layer.disabled.is_empty() {
            parts.push(format!(
                "{} {}",
                dialog.draft().layer.disabled.len(),
                i18n.text("prompt-studio-hidden")
            ));
        }
        if !dialog.draft().models.is_empty() {
            parts.push(format!(
                "{} {}",
                dialog.draft().models.len(),
                i18n.text("prompt-studio-model-layers")
            ));
        }
        parts.join("  ·  ")
    } else {
        sanitize_terminal_text(&dialog.status)
    };
    let status_height = if status.is_empty() {
        0
    } else {
        let max = if body.height >= 10 { 2 } else { 1 };
        Paragraph::new(status.as_str())
            .wrap(Wrap { trim: true })
            .line_count(body.width)
            .min(max) as u16
    };
    let [context_area, content, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(status_height),
    ])
    .areas(body);
    frame.render_widget(context, context_area);
    if status_height > 0 {
        frame.render_widget(
            Paragraph::new(status)
                .style(theme::muted_style())
                .wrap(Wrap { trim: true }),
            status_area,
        );
    }
    let (list_area, detail_area) = if content.width >= 73 && row.is_some() {
        let [list, detail] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(content);
        let border = Block::new()
            .borders(Borders::LEFT)
            .border_style(theme::muted_style());
        let inner = border.inner(detail).inner(Margin::new(1, 0));
        frame.render_widget(border, detail);
        (list, Some(inner))
    } else {
        (content, None)
    };
    let [list_header, list_body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(list_area);
    frame.render_widget(
        Line::styled(
            format!("{}  {}", i18n.text(heading), rows.len()),
            theme::modal_border_style(),
        ),
        list_header,
    );
    if rows.is_empty() {
        let message = if dialog.preview_request.is_some() {
            "prompt-studio-loading"
        } else {
            "prompt-studio-empty"
        };
        frame.render_widget(
            Paragraph::new(i18n.text(message))
                .wrap(Wrap { trim: true })
                .style(theme::muted_style()),
            list_body,
        );
        return;
    }
    render_rows(frame, list_body, &rows, selected);
    if let (Some(area), Some(row)) = (detail_area, row) {
        render_detail(frame, area, row, None, i18n);
    }
}

fn scope_line(dialog: &PromptStudioOverlay, i18n: &I18n, width: u16) -> Line<'static> {
    let scopes = [
        (PromptStudioScope::Global, "prompt-studio-global"),
        (PromptStudioScope::Workspace, "prompt-studio-workspace"),
        (PromptStudioScope::Session, "prompt-studio-session"),
    ];
    let compact = width < 55;
    let mut spans = Vec::new();
    for (scope, key) in scopes {
        if scope == dialog.scope {
            spans.push(Span::styled(
                format!(" {} ", i18n.text(key)),
                theme::selection_style(),
            ));
        } else if !compact {
            spans.push(Span::styled(
                format!(" {} ", i18n.text(key)),
                theme::muted_style(),
            ));
        }
    }
    let mode = match dialog.draft().layer.mode {
        LayerMode::Append => "prompt-studio-append",
        LayerMode::Replace => "prompt-studio-replace",
    };
    spans.push(Span::styled(
        format!("  {}", i18n.text(mode)),
        theme::muted_style(),
    ));
    if dialog.draft() != &dialog.saved[dialog.scope_index()] {
        spans.push(Span::styled(
            format!("  ·  {}", i18n.text("prompt-studio-unsaved")),
            Style::default().fg(theme::warning_color()),
        ));
    }
    Line::from(spans)
}

pub(crate) fn render_rows(frame: &mut Frame, area: Rect, rows: &[PromptRow], selected: usize) {
    if area.height == 0 || rows.is_empty() {
        return;
    }
    let highlight = Line::raw("› ");
    let content_width = usize::from(area.width).saturating_sub(highlight.width());
    let row_height = area.height.min(2);
    let items = rows.iter().map(|row| {
        let order = row
            .position
            .map_or_else(String::new, |position| format!("{position:>2} "));
        let indent = (order.len() + 2).min(content_width.saturating_sub(1));
        let mut lines = vec![Line::from(vec![
            Span::styled(order, theme::muted_style()),
            Span::styled(
                if row.builtin { "◆ " } else { "  " },
                Style::default().fg(theme::accent_color()),
            ),
            Span::raw(&row.title),
        ])];
        if row_height == 2 {
            let excerpt = if row.excerpt.is_empty() {
                &row.description
            } else {
                &row.excerpt
            };
            let text = truncate_display_text_with_suffix(excerpt, content_width - indent, "…");
            lines.push(Line::styled(
                format!("{}{text}", " ".repeat(indent)),
                theme::muted_style(),
            ));
        }
        ListItem::new(lines)
    });
    let mut state = ListState::default().with_selected(Some(selected.min(rows.len() - 1)));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol(highlight)
            .highlight_style(theme::selection_style()),
        area,
        &mut state,
    );
    let panel = pointer::next_list();
    let keys = Some((pointer::key(KeyCode::Up), pointer::key(KeyCode::Down)));
    pointer::register(
        area,
        Some(pointer::PointerAction::FocusList(panel)),
        keys.clone(),
    );
    for (visible, index) in (state.offset()..rows.len())
        .take(usize::from(area.height / row_height))
        .enumerate()
    {
        pointer::register(
            Rect::new(
                area.x,
                area.y + visible as u16 * row_height,
                area.width,
                row_height,
            ),
            Some(pointer::PointerAction::List {
                panel,
                index,
                selected,
            }),
            keys.clone(),
        );
    }
}

pub(crate) fn render_detail(
    frame: &mut Frame,
    area: Rect,
    row: &PromptRow,
    scroll: Option<&Cell<u16>>,
    i18n: &I18n,
) {
    if area.is_empty() {
        return;
    }
    let detail_title = if row.builtin {
        format!("◆ {}", row.title)
    } else {
        row.title.clone()
    };
    let mut lines = vec![
        Line::styled(
            detail_title,
            Style::default()
                .fg(theme::accent_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            if row.builtin {
                format!(
                    "{}  ·  {}",
                    i18n.text("prompt-studio-built-in"),
                    row.description
                )
            } else {
                row.description.clone()
            },
            theme::muted_style(),
        ),
        Line::default(),
    ];
    lines.extend(row.body.lines().map(|line| {
        if line.starts_with("# ") || line.starts_with("## ") {
            Line::styled(line, Style::default().add_modifier(Modifier::BOLD))
        } else {
            Line::from(line)
        }
    }));
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let max = paragraph
        .line_count(area.width)
        .saturating_sub(usize::from(area.height));
    let offset = if let Some(scroll) = scroll {
        let offset = usize::from(scroll.get()).min(max) as u16;
        scroll.set(offset);
        offset
    } else {
        0
    };
    frame.render_widget(paragraph.scroll((offset, 0)), area);
    if scroll.is_some() && max > 0 {
        let mut state = ScrollbarState::new(max + 1).position(usize::from(offset));
        let rail = Rect::new(area.right(), area.y, 1, area.height);
        if rail.right() <= frame.area().right() {
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                rail,
                &mut state,
            );
        }
        pointer::register(
            area,
            None,
            Some((pointer::key(KeyCode::Up), pointer::key(KeyCode::Down))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn both_lines_target_the_visible_prompt_after_scrolling() {
        let body = format!("# 标题\n\n{}\nFull detail", "查阅正文".repeat(30));
        let rows: Vec<_> = (0..5)
            .map(|index| PromptRow {
                title: format!("Prompt {index}"),
                description: "Saved prompt".into(),
                excerpt: prompt_excerpt(&body, false),
                body: body.clone(),
                position: Some(index + 1),
                builtin: false,
            })
            .collect();
        let mut terminal = Terminal::new(TestBackend::new(24, 5)).unwrap();
        let targets = pointer::capture(|| {
            terminal
                .draw(|frame| render_rows(frame, frame.area(), &rows, 4))
                .unwrap();
        });
        let line = |y| {
            let buffer = terminal.backend().buffer();
            (0..24).map(|x| buffer[(x, y)].symbol()).collect::<String>()
        };
        assert!(line(0).contains("Prompt 3"));
        assert!(line(2).contains("Prompt 4"));
        for y in [1, 3] {
            let excerpt = line(y).replace(' ', "");
            assert!(
                excerpt.starts_with("查阅正文") && excerpt.ends_with('…'),
                "{excerpt}"
            );
        }
        let click = |row| {
            targets.action(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 8,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        for (row, index) in [(0, 3), (1, 3), (2, 4), (3, 4)] {
            assert_eq!(
                click(row),
                Some(pointer::PointerAction::List {
                    panel: 0,
                    index,
                    selected: 4
                }),
            );
        }
        assert_eq!(click(4), Some(pointer::PointerAction::FocusList(0)));
    }
}
