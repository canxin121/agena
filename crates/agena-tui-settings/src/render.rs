//! Settings-specific layout. Chat and other workbench surfaces keep their own chrome.

use agena_tui::i18n::I18n;
use agena_tui_components::{
    SurfaceMode,
    pointer::{self, PointerAction},
    sanitize_editor_text, theme, truncate_display_text,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{SettingsStudioFocus, SettingsStudioPresentation, section_group_label};

pub struct SettingsViewSpec<'a> {
    pub title: &'a str,
    pub detail_title: &'a str,
    pub detail: Text<'a>,
}

/// Render a settings page with a category rail and readable, responsive rows.
/// The only interior state updated here is viewport geometry for paging/scrolling.
pub fn render_settings<A>(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    state: &SettingsStudioPresentation<A>,
    i18n: &I18n,
    spec: &SettingsViewSpec<'_>,
) {
    pointer::begin_surface();
    let area = surface.outer_rect(area, 150, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    if area.is_empty() {
        return;
    }
    let inner = area.inner(Margin::new(u16::from(area.width > 6) * 2, 0));
    let header_height = if inner.height >= 12 { 4 } else { 1 };
    let footer_height = if inner.height > 3 { 2 } else { 0 };
    let header = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        header_height.min(inner.height),
    );
    let body = Rect::new(
        inner.x,
        header.bottom(),
        inner.width,
        inner.height.saturating_sub(header.height + footer_height),
    );
    let footer = Rect::new(inner.x, body.bottom(), inner.width, footer_height);

    render_header(frame, header, state, i18n, spec.title);
    let wide = body.width >= 76;
    if wide {
        let nav_width = (body.width / 4).clamp(23, 30);
        let navigation = Rect::new(body.x, body.y, nav_width, body.height);
        let content = Rect::new(
            navigation.right() + 3,
            body.y,
            body.width.saturating_sub(nav_width + 3),
            body.height,
        );
        render_navigation(frame, navigation, state, i18n);
        render_items(frame, content, state, i18n);
    } else if state.focus() == SettingsStudioFocus::Navigation && !state.searching {
        render_navigation(frame, body, state, i18n);
    } else {
        render_items(frame, body, state, i18n);
    }
    render_footer(frame, footer, state, i18n);
    if state.details {
        render_details(frame, area, state, i18n, spec);
    }
}

fn ctrl(code: char) -> PointerAction {
    PointerAction::Key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL))
}

fn clean(value: &str) -> String {
    sanitize_editor_text(value).replace(['\n', '\r', '\t'], " ")
}

fn line(frame: &mut Frame, area: Rect, text: impl Into<String>, style: Style) {
    if !area.is_empty() {
        frame.render_widget(
            Paragraph::new(truncate_display_text(
                &clean(&text.into()),
                area.width as usize,
            ))
            .style(style),
            area,
        );
    }
}

fn row(area: Rect, offset: u16) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        u16::from(offset < area.height),
    )
}

fn render_header<A>(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsStudioPresentation<A>,
    i18n: &I18n,
    title: &str,
) {
    let muted = theme::muted_style();
    let title_width = if area.width >= 76 {
        area.width.saturating_sub(43)
    } else {
        area.width
    };
    let title_area = Rect::new(
        area.x,
        area.y + u16::from(area.height > 1),
        title_width,
        u16::from(area.height > 0),
    );
    let mut title = vec![Span::styled(
        clean(title),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(section) = state.selected_section() {
        title.push(Span::styled(
            format!("  /  {}", clean(&section.label)),
            muted,
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), title_area);
    let search_area = if area.width >= 76 && area.height >= 3 {
        Rect::new(area.right().saturating_sub(40), area.y, 40, 3)
    } else if area.height >= 4 {
        Rect::new(area.x, area.y + 2, area.width, 1)
    } else {
        Rect::default()
    };
    if !search_area.is_empty() {
        let mut input = search_area;
        if search_area.height == 3 {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(if state.searching {
                    Style::default().fg(theme::accent_color())
                } else {
                    muted
                });
            input = block.inner(search_area);
            frame.render_widget(block, search_area);
        }
        let prefix = "/ ";
        line(frame, input, prefix, muted);
        input.x += 2.min(input.width);
        input.width = input.width.saturating_sub(3);
        if state.query().is_empty() && !state.searching {
            line(
                frame,
                input,
                i18n.text("settings-search-placeholder"),
                muted,
            );
        } else {
            let view = state.search.render_view(input.width, 1);
            frame.render_widget(Paragraph::new(view.lines), input);
            if state.searching && !input.is_empty() {
                frame.set_cursor_position((input.x + view.cursor_x, input.y));
            }
        }
        pointer::register(search_area, Some(ctrl('f')), None);
    }
}

fn render_navigation<A>(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsStudioPresentation<A>,
    i18n: &I18n,
) {
    if area.is_empty() {
        return;
    }
    let bordered = area.width <= 30;
    let block = Block::default()
        .borders(if bordered {
            Borders::RIGHT
        } else {
            Borders::NONE
        })
        .border_style(theme::muted_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    pointer::register(inner, Some(PointerAction::FocusList(0)), None);
    let indices = state.visible_section_indices();
    if state.focus() == SettingsStudioFocus::Navigation {
        state.page_size.set(usize::from(inner.height).max(1));
    }
    if indices.is_empty() {
        line(
            frame,
            inner,
            i18n.text("settings-search-empty"),
            theme::muted_style(),
        );
        return;
    }
    // Group labels are decorative rows; hit targets retain filtered category indices.
    let mut rows: Vec<(Option<usize>, String)> = Vec::new();
    let grouped = inner.height as usize >= indices.len() + 7;
    let mut group = String::new();
    for (position, &index) in indices.iter().enumerate() {
        let section = &state.sections()[index];
        let next_group = section_group_label(i18n, section.id);
        if grouped && next_group != group {
            if !rows.is_empty() {
                rows.push((None, String::new()));
            }
            rows.push((None, next_group.clone()));
            group = next_group;
        }
        rows.push((Some(position), section.label.clone()));
    }
    let selected = indices
        .iter()
        .position(|&index| index == state.selected_section_index())
        .unwrap_or(0);
    let selected_row = rows
        .iter()
        .position(|(index, _)| *index == Some(selected))
        .unwrap_or(0);
    let offset = selected_row.saturating_sub(inner.height.saturating_sub(1) as usize);
    for (y, (position, label)) in rows
        .iter()
        .skip(offset)
        .take(inner.height as usize)
        .enumerate()
    {
        let target = row(inner, y as u16);
        let Some(position) = position else {
            line(frame, target, format!("  {label}"), theme::muted_style());
            continue;
        };
        let active = *position == selected;
        let focused = state.focus() == SettingsStudioFocus::Navigation && !state.searching;
        let style = if active && focused {
            theme::selection_style()
        } else if active {
            Style::default()
                .fg(theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let count = state
            .visible_item_indices(indices[*position])
            .len()
            .to_string();
        let label_width = target.width.saturating_sub(count.len() as u16 + 5);
        let label = truncate_display_text(&clean(label), label_width as usize);
        let padding = (label_width as usize).saturating_sub(UnicodeWidthStr::width(label.as_str()));
        line(
            frame,
            target,
            format!(
                "{} {}{}  {} ",
                if active { "›" } else { " " },
                label,
                " ".repeat(padding),
                count
            ),
            style,
        );
        pointer::register(
            target,
            Some(PointerAction::List {
                panel: 0,
                index: *position,
                selected,
            }),
            None,
        );
    }
}

fn render_items<A>(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsStudioPresentation<A>,
    i18n: &I18n,
) {
    if area.is_empty() {
        return;
    }
    let indices = state.visible_item_indices(state.selected_section_index());
    let Some(section) = state.selected_section() else {
        return;
    };
    if indices.is_empty() {
        let key = if state.query().trim().is_empty() {
            "overlay-settings-empty-items"
        } else {
            "settings-search-empty"
        };
        line(
            frame,
            row(area, 1),
            i18n.text(key),
            Style::default().add_modifier(Modifier::BOLD),
        );
        if !state.query().trim().is_empty() {
            frame.render_widget(
                Paragraph::new(i18n.text("settings-search-empty-hint"))
                    .style(theme::muted_style())
                    .wrap(Wrap { trim: false }),
                Rect::new(
                    area.x,
                    area.y + 3,
                    area.width,
                    area.height.saturating_sub(3),
                ),
            );
        }
        return;
    }
    let selected = indices
        .iter()
        .position(|&index| index == state.selected_item_index())
        .unwrap_or(0);
    let count = format!("{} / {}", selected + 1, indices.len());
    let title_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(count.len() as u16 + 2),
        1,
    );
    line(
        frame,
        title_area,
        &section.label,
        Style::default().add_modifier(Modifier::BOLD),
    );
    let count_width = (count.len() as u16).min(area.width);
    frame.render_widget(
        Paragraph::new(count)
            .alignment(Alignment::Right)
            .style(theme::muted_style()),
        Rect::new(area.right() - count_width, area.y, count_width, 1),
    );
    let heading_height = if area.height >= 10 {
        4
    } else {
        2.min(area.height)
    };
    if heading_height == 4 {
        frame.render_widget(
            Paragraph::new(clean(&section.description))
                .style(theme::muted_style())
                .wrap(Wrap { trim: false }),
            Rect::new(area.x, area.y + 1, area.width, 2),
        );
    }
    let list = Rect::new(
        area.x,
        area.y + heading_height,
        area.width,
        area.height.saturating_sub(heading_height),
    );
    pointer::register(list, Some(PointerAction::FocusList(1)), None);
    let stacked = list.width < 58;
    let row_height = if stacked { 4 } else { 3 };
    let capacity = (list.height / row_height).max(1) as usize;
    if state.focus() == SettingsStudioFocus::Items {
        state.page_size.set(capacity);
    }
    let offset = selected.saturating_sub(capacity.saturating_sub(1));
    for (position, &index) in indices.iter().enumerate().skip(offset).take(capacity) {
        let y = list.y + ((position - offset) as u16 * row_height);
        if y >= list.bottom() {
            break;
        }
        let target = Rect::new(list.x, y, list.width, row_height.min(list.bottom() - y));
        let item = &section.items[index];
        let active = position == selected;
        let focused = state.focus() == SettingsStudioFocus::Items && !state.searching;
        let palette = theme::active_palette();
        let style = if active && focused {
            theme::modal_surface_style()
        } else {
            Style::default()
        };
        let highlight = Rect::new(
            target.x,
            target.y,
            target.width,
            target.height.min(if stacked { 3 } else { 2 }),
        );
        frame.render_widget(Block::default().style(style), highlight);
        if active {
            line(
                frame,
                Rect::new(target.x, target.y, 1, target.height),
                "▎",
                Style::default().fg(palette.accent),
            );
        }
        let content = Rect::new(
            target.x + 2.min(target.width),
            target.y,
            target.width.saturating_sub(5),
            target.height,
        );
        let value = match item.value.as_str() {
            "true" => format!("● {}", i18n.text("value-on")),
            "false" => format!("○ {}", i18n.text("value-off")),
            _ => clean(&item.value),
        };
        let value_style = if item.value == "true" && !(active && focused) {
            Style::default().fg(palette.success)
        } else if active && focused {
            Style::default().fg(palette.modal_border)
        } else {
            Style::default().fg(palette.accent)
        };
        if stacked {
            line(
                frame,
                row(content, 0),
                &item.label,
                style.add_modifier(Modifier::BOLD),
            );
            line(frame, row(content, 1), &value, value_style);
        } else {
            let value_width = (content.width * 2 / 5).max(12);
            let label = Rect::new(
                content.x,
                content.y,
                content.width.saturating_sub(value_width + 2),
                1,
            );
            line(
                frame,
                label,
                &item.label,
                style.add_modifier(Modifier::BOLD),
            );
            let value_area = Rect::new(content.right() - value_width, content.y, value_width, 1);
            frame.render_widget(
                Paragraph::new(truncate_display_text(&value, value_width as usize))
                    .alignment(Alignment::Right)
                    .style(value_style),
                value_area,
            );
        }
        // The row preview stays compact; the complete text and configuration
        // sources are available in the scrollable details view.
        let description_style = if active && focused {
            Style::default().fg(palette.modal_fg)
        } else {
            theme::muted_style()
        };
        line(
            frame,
            row(content, if stacked { 2 } else { 1 }),
            &item.detail,
            description_style,
        );
        if target.width > 4 {
            line(
                frame,
                Rect::new(target.right() - 2, target.y, 1, 1),
                "›",
                value_style,
            );
        }
        pointer::register(
            target,
            Some(PointerAction::List {
                panel: 1,
                index: position,
                selected,
            }),
            None,
        );
    }
    if indices.len() > capacity && list.width > 0 && list.height > 1 {
        let thumb = ((list.height as usize * capacity) / indices.len()).max(1) as u16;
        let travel = list.height.saturating_sub(thumb);
        let y = list.y
            + (travel as usize * offset / indices.len().saturating_sub(capacity).max(1)) as u16;
        frame.render_widget(
            Paragraph::new("│\n".repeat(list.height as usize)).style(theme::muted_style()),
            Rect::new(list.right() - 1, list.y, 1, list.height),
        );
        frame.render_widget(
            Paragraph::new("┃\n".repeat(thumb as usize))
                .style(Style::default().fg(theme::accent_color())),
            Rect::new(list.right() - 1, y, 1, thumb),
        );
    }
}

fn render_footer<A>(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsStudioPresentation<A>,
    i18n: &I18n,
) {
    if area.is_empty() {
        return;
    }
    let inner = if area.height > 1 {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(theme::muted_style());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };
    let back_label = i18n.text(if state.query().is_empty() && !state.searching {
        "settings-key-back"
    } else {
        "settings-key-clear"
    });
    let back_width = (UnicodeWidthStr::width(back_label.as_str()) + 4) as u16;
    let back_width = back_width.min(inner.width);
    let back = Rect::new(inner.right() - back_width, inner.y, back_width, 1);
    render_shortcut(frame, back, "Esc", &back_label, pointer::key(KeyCode::Esc));
    let keys = if state.searching {
        vec![(
            "Enter",
            "settings-key-results",
            pointer::key(KeyCode::Enter),
        )]
    } else {
        vec![
            (
                "Tab",
                if state.focus() == SettingsStudioFocus::Navigation {
                    "settings-key-items"
                } else {
                    "settings-key-sections"
                },
                pointer::key(KeyCode::Tab),
            ),
            ("Enter", "settings-key-open", pointer::key(KeyCode::Enter)),
            ("/", "settings-key-search", ctrl('f')),
            (
                if inner.width >= 96 { "Ctrl+D" } else { "^D" },
                "settings-key-details",
                ctrl('d'),
            ),
            (
                if inner.width >= 96 { "Ctrl+R" } else { "^R" },
                "settings-key-refresh",
                ctrl('r'),
            ),
        ]
    };
    let mut x = inner.x;
    for (key, label, action) in keys {
        let label = i18n.text(label);
        let width =
            (UnicodeWidthStr::width(key) + UnicodeWidthStr::width(label.as_str()) + 3) as u16;
        if x + width > back.x.saturating_sub(1) {
            break;
        }
        render_shortcut(frame, Rect::new(x, inner.y, width, 1), key, &label, action);
        x += width;
    }
}

fn render_shortcut(frame: &mut Frame, area: Rect, key: &str, label: &str, action: PointerAction) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                key.to_owned(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {label}"), theme::muted_style()),
        ])),
        area,
    );
    pointer::register(area, Some(action), None);
}

fn render_details<A>(
    frame: &mut Frame,
    area: Rect,
    state: &SettingsStudioPresentation<A>,
    i18n: &I18n,
    spec: &SettingsViewSpec<'_>,
) {
    pointer::begin_surface();
    let width = area.width.saturating_sub(4).clamp(1, 90).min(area.width);
    let height = area.height.saturating_sub(2).max(1).min(area.height);
    let outer = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, outer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(theme::modal_surface_style())
        .border_style(theme::modal_border_style())
        .title(format!(
            " {} ",
            truncate_display_text(&clean(spec.detail_title), width.saturating_sub(4) as usize)
        ));
    let inner = block
        .inner(outer)
        .inner(Margin::new(u16::from(width > 8), 0));
    frame.render_widget(block, outer);
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let paragraph = Paragraph::new(spec.detail.clone()).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(body.width)
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    state.detail_max_scroll.set(max_scroll);
    state.page_size.set(body.height.max(1) as usize);
    let scroll = state.detail_scroll.get().min(max_scroll);
    state.detail_scroll.set(scroll);
    frame.render_widget(paragraph.scroll((scroll, 0)), body);
    let footer = Rect::new(
        inner.x,
        body.bottom().saturating_add(1),
        inner.width,
        u16::from(inner.height >= 2),
    );
    line(
        frame,
        footer,
        i18n.text("settings-details-footer"),
        theme::modal_surface_style(),
    );
    pointer::register(footer, Some(pointer::key(KeyCode::Esc)), None);
    pointer::register(
        body,
        None,
        Some((pointer::key(KeyCode::Up), pointer::key(KeyCode::Down))),
    );
}
