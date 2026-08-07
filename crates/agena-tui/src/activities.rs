//! Presentation vocabulary for the unified background-activities panel.
//!
//! Fetching activity data and performing stop/dismiss effects remain
//! application responsibilities. This module owns the terminal projection:
//! row grouping/ordering, selection, filters, and rendering. It deliberately
//! does not depend on the API wire crate — the application projects its
//! resources into [`ActivitiesRow`] before rendering, mirroring `usage`.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use agena_tui_components::theme::{
    accent_color, danger_color, muted_style, selection_style, success_color, warning_color,
};

/// Terminal control activated by the user inside the activities panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitiesControl {
    MoveUp,
    MoveDown,
    ToggleDetail,
    ToggleFinished,
    CycleKindFilter,
    CycleStatusFilter,
    Refresh,
    Stop,
    Dismiss,
    ClearFinished,
    Close,
}

/// Intent returned to the application adapter so it can trigger effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivitiesEffect {
    None,
    Reload,
    Stop(String),
    Dismiss(String),
    ClearFinished,
    ToggleDetail,
}

/// Display-only row projection of one background activity.
#[derive(Debug, Clone)]
pub struct ActivitiesRow {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    pub session_id: Option<i64>,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub exit_code: Option<i32>,
    pub message: Option<String>,
    pub cancellable: bool,
    pub dismissible: bool,
}

impl ActivitiesRow {
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "running")
    }

    pub fn running_seconds(&self, now_ms: i64) -> Option<i64> {
        let start = self.started_at_ms;
        let end = self.finished_at_ms.unwrap_or(now_ms);
        (end >= start).then_some((end - start) / 1000)
    }
}

/// Pure panel presentation state. Runtime results and navigation effects stay
/// in the application adapter.
#[derive(Debug, Clone)]
pub struct ActivitiesPresentation {
    pub selected: usize,
    pub scroll: usize,
    pub detail: bool,
    pub show_finished: bool,
    pub kind_filter: Option<String>,
    pub status_filter: Option<String>,
}

impl Default for ActivitiesPresentation {
    fn default() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            detail: false,
            show_finished: true,
            kind_filter: None,
            status_filter: None,
        }
    }
}

/// Ordering used by the list: active rows first, then newest start time.
pub fn sort_rows(rows: &mut [ActivitiesRow]) {
    rows.sort_by(|a, b| {
        b.is_active()
            .cmp(&a.is_active())
            .then_with(|| b.started_at_ms.cmp(&a.started_at_ms))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Rows visible under the current filters, in display order.
pub fn visible_rows<'a>(
    rows: &'a [ActivitiesRow],
    filter: &ActivitiesPresentation,
) -> Vec<&'a ActivitiesRow> {
    rows.iter()
        .filter(|row| {
            if !filter.show_finished && row.is_active() == false {
                return false;
            }
            if let Some(kind) = filter.kind_filter.as_deref() {
                if row.kind != kind {
                    return false;
                }
            }
            if let Some(status) = filter.status_filter.as_deref() {
                if row.status != status {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Line offset of each visible row in the rendered list, mirroring the
/// renderer's section-header layout exactly. Used to keep the selected row
/// visible while scrolling and to page by whole screens.
pub fn row_line_offsets(visible: &[&ActivitiesRow]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(visible.len());
    let mut line = 0;
    let mut last_section: Option<&'static str> = None;
    for row in visible {
        let section = if row.is_active() {
            "Active"
        } else {
            "Finished"
        };
        if last_section != Some(section) {
            line += 2;
            last_section = Some(section);
        }
        offsets.push(line);
        line += 1;
    }
    offsets
}

impl ActivitiesPresentation {
    pub fn move_selection(&mut self, row_count: usize, delta: isize) {
        if row_count == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, row_count as isize - 1) as usize;
    }

    pub fn clamp_selection(&mut self, row_count: usize) {
        if row_count == 0 {
            self.selected = 0;
            self.scroll = 0;
        } else {
            self.selected = self.selected.min(row_count - 1);
        }
    }

    /// Scroll the panel so the selected row's rendered line is inside the
    /// viewport. `offsets` comes from [`row_line_offsets`]; `viewport_lines` is
    /// the visible list height in rendered lines. `scroll` is kept in row
    /// units, matching the first visible row.
    pub fn reveal_selected(&mut self, offsets: &[usize], viewport_lines: usize) {
        if offsets.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        self.selected = self.selected.min(offsets.len().saturating_sub(1));
        self.scroll = self.scroll.min(offsets.len().saturating_sub(1));
        let viewport = viewport_lines.max(1);
        let target = offsets[self.selected];
        let current = offsets[self.scroll];
        if target < current {
            // The selected row moved above the viewport; put it at the top.
            self.scroll = self.selected;
        } else if target >= current.saturating_add(viewport) {
            // The selected row moved below the viewport; scroll down just
            // enough to bring its line into view.
            let min_visible = target.saturating_sub(viewport).saturating_add(1);
            let mut row = self.scroll;
            while row + 1 < offsets.len() && offsets[row] < min_visible {
                row += 1;
            }
            self.scroll = row;
        }
    }

    /// Cycle the kind filter across `shell`, `task`, `runtime`, `browser`, none.
    pub fn cycle_kind_filter(&mut self) {
        self.kind_filter = match self.kind_filter.as_deref() {
            None => Some("shell".to_owned()),
            Some("shell") => Some("task".to_owned()),
            Some("task") => Some("runtime".to_owned()),
            Some("runtime") => Some("browser".to_owned()),
            _ => None,
        };
        self.selected = 0;
        self.scroll = 0;
    }

    /// Cycle the status filter across running, pending, failed, succeeded, none.
    pub fn cycle_status_filter(&mut self) {
        self.status_filter = match self.status_filter.as_deref() {
            None => Some("running".to_owned()),
            Some("running") => Some("pending".to_owned()),
            Some("pending") => Some("failed".to_owned()),
            Some("failed") => Some("succeeded".to_owned()),
            _ => None,
        };
        self.selected = 0;
        self.scroll = 0;
    }
}

/// Log tail projection shown in the detail pane.
#[derive(Debug, Clone, Default)]
pub struct ActivitiesLogTail {
    pub lines: Vec<String>,
    pub last_seq: u64,
    pub has_more: bool,
    pub dropped_lines: u64,
}

/// A stable label used for filter chips and section headers.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "shell" => "Shell",
        "task" => "Task",
        "runtime" => "Runtime",
        "browser" => "Browser",
        _ => "Activity",
    }
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "pending" => "pending",
        "running" => "running",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "stopped" => "stopped",
        _ => "unknown",
    }
}

pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "shell" => "\u{2699}",   // ⚙
        "task" => "\u{25C8}",    // ◈
        "runtime" => "\u{21BB}", // ↻
        "browser" => "\u{25C9}", // ◉
        _ => "\u{2022}",         // •
    }
}

fn status_style(status: &str) -> Style {
    match status {
        "running" | "pending" => Style::default().fg(accent_color()),
        "succeeded" => Style::default().fg(success_color()),
        "failed" => Style::default().fg(danger_color()),
        "cancelled" | "stopped" => Style::default().fg(warning_color()),
        _ => muted_style(),
    }
}

fn kind_style(kind: &str) -> Style {
    match kind {
        "shell" => Style::default().fg(info_color()),
        "task" => Style::default().fg(special_color()),
        "runtime" => Style::default().fg(warning_color()),
        "browser" => Style::default().fg(accent_color()),
        _ => muted_style(),
    }
}

fn info_color() -> ratatui::style::Color {
    agena_tui_components::theme::info_color()
}

fn special_color() -> ratatui::style::Color {
    agena_tui_components::theme::special_color()
}

/// Render the complete activities panel into `area`.
pub fn render_activities_panel(
    frame: &mut Frame,
    area: Rect,
    presentation: &ActivitiesPresentation,
    rows: &[ActivitiesRow],
    loading: bool,
    error: Option<&str>,
    log_tail: Option<&ActivitiesLogTail>,
    now_ms: i64,
) {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let list_area = horizontal[0];
    let detail_area = if presentation.detail {
        horizontal[1]
    } else {
        Rect::default()
    };

    render_list_pane(frame, list_area, presentation, rows, loading, error, now_ms);
    if presentation.detail {
        render_detail_pane(frame, detail_area, presentation, rows, log_tail);
    }
}

fn render_list_pane(
    frame: &mut Frame,
    area: Rect,
    presentation: &ActivitiesPresentation,
    rows: &[ActivitiesRow],
    loading: bool,
    error: Option<&str>,
    now_ms: i64,
) {
    let visible = visible_rows(rows, presentation);
    let active_count = visible.iter().filter(|row| row.is_active()).count();
    let finished_count = visible.len().saturating_sub(active_count);

    let title = format!(
        " Background Activities {}{} ",
        if loading { "…" } else { "" },
        filter_suffix(presentation)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(muted_style())
        .title(Line::from(Span::styled(title, Style::default().add_modifier(Modifier::BOLD))))
        .title_bottom(Line::from(format!(
            " {active_count} active · {finished_count} finished | ↑↓ select  PgUp/PgDn page  ↵ detail  s stop  d dismiss  x clear  r refresh  q close "
        )));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!(" ✗ {error}"),
            Style::default().fg(danger_color()),
        )));
        lines.push(Line::from(""));
    } else if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            " No background activities.",
            muted_style(),
        )));
        lines.push(Line::from(Span::styled(
            " Run `shell.run background` or `tasks.run` to create one.",
            muted_style(),
        )));
    } else {
        let mut index = 0_usize;
        let mut last_section: Option<&'static str> = None;
        for row in &visible {
            let section = if row.is_active() {
                "Active"
            } else {
                "Finished"
            };
            if last_section != Some(section) {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!(" {section}"),
                    Style::default()
                        .fg(warning_color())
                        .add_modifier(Modifier::BOLD),
                )));
                last_section = Some(section);
            }
            lines.push(render_row_line(row, index == presentation.selected, now_ms));
            index += 1;
        }
    }

    let offsets = row_line_offsets(&visible);
    let scroll_row = presentation.scroll.min(offsets.len().saturating_sub(1));
    let scroll_line = offsets
        .get(scroll_row)
        .copied()
        .unwrap_or(0)
        .min(lines.len().saturating_sub(inner.height as usize));
    let visible_lines: Vec<Line<'static>> = lines
        .iter()
        .skip(scroll_line)
        .take(inner.height as usize)
        .cloned()
        .collect();
    frame.render_widget(
        Paragraph::new(visible_lines).wrap(Wrap { trim: false }),
        inner,
    );
}

fn render_row_line(row: &ActivitiesRow, selected: bool, now_ms: i64) -> Line<'static> {
    let icon = Span::styled(format!(" {} ", kind_icon(&row.kind)), kind_style(&row.kind));
    let status = Span::styled(
        format!("{:<9}", status_label(&row.status)),
        status_style(&row.status),
    );
    let duration = row
        .running_seconds(now_ms)
        .map(|secs| format!("{:>5}s", secs))
        .unwrap_or_else(|| "   –  ".to_owned());
    let duration = Span::styled(duration, muted_style());

    let mut title_text = row.title.clone();
    if !row.description.is_empty() && row.description != row.title {
        title_text.push_str(" — ");
        title_text.push_str(&row.description);
    }
    let title = Span::styled(title_text, Style::default().add_modifier(Modifier::BOLD));

    let mut spans = vec![icon, status, duration, Span::raw(" "), title];
    if let Some(command) = row.command.as_deref() {
        spans.push(Span::styled(format!("  ({command})"), muted_style()));
    }
    if let Some(exit) = row.exit_code {
        spans.push(Span::styled(format!(" exit={exit}"), muted_style()));
    }
    if let Some(message) = row.message.as_deref() {
        spans.push(Span::styled(format!(" · {message}"), muted_style()));
    }

    let line = Line::from(spans);
    if selected {
        line.style(selection_style())
    } else {
        line
    }
}

fn render_detail_pane(
    frame: &mut Frame,
    area: Rect,
    presentation: &ActivitiesPresentation,
    rows: &[ActivitiesRow],
    log_tail: Option<&ActivitiesLogTail>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(muted_style())
        .title(Line::from(Span::styled(
            " Detail / Log tail ",
            Style::default().add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = visible_rows(rows, presentation);
    let Some(selected) = visible.get(presentation.selected) else {
        frame.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(
                " No selection.",
                muted_style(),
            ))]),
            inner,
        );
        return;
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("{} {}", kind_icon(&selected.kind), selected.title),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(selected.id.clone(), muted_style())));
    if !selected.description.is_empty() {
        lines.push(Line::from(Span::styled(
            selected.description.clone(),
            muted_style(),
        )));
    }
    if let Some(command) = selected.command.as_deref() {
        lines.push(Line::from(Span::styled(
            format!(" $ {command}"),
            muted_style(),
        )));
    }
    if let Some(session_id) = selected.session_id {
        lines.push(Line::from(Span::styled(
            format!(" session #{session_id}"),
            muted_style(),
        )));
    }
    if let Some(message) = selected.message.as_deref() {
        lines.push(Line::from(Span::styled(
            format!(" {message}"),
            status_style(&selected.status),
        )));
    }
    lines.push(Line::from(""));

    match log_tail {
        Some(tail) if !tail.lines.is_empty() => {
            lines.push(Line::from(Span::styled(
                " — output — ",
                Style::default()
                    .fg(warning_color())
                    .add_modifier(Modifier::BOLD),
            )));
            let skipped = tail.dropped_lines;
            for line in tail
                .lines
                .iter()
                .rev()
                .take(inner.height.saturating_sub(6) as usize)
            {
                lines.push(Line::from(Span::styled(line.clone(), Style::default())));
            }
            if tail.has_more {
                lines.push(Line::from(Span::styled(
                    format!(" … {skipped} older lines dropped"),
                    muted_style(),
                )));
            }
        }
        Some(_) => {
            lines.push(Line::from(Span::styled(" No output yet.", muted_style())));
        }
        None => {
            lines.push(Line::from(Span::styled(
                " Press ↵ to tail logs for this activity.",
                muted_style(),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn filter_suffix(presentation: &ActivitiesPresentation) -> String {
    let mut chips: Vec<String> = Vec::new();
    if !presentation.show_finished {
        chips.push("active only".to_owned());
    }
    if let Some(kind) = presentation.kind_filter.as_deref() {
        chips.push(kind.to_owned());
    }
    if let Some(status) = presentation.status_filter.as_deref() {
        chips.push(status.to_owned());
    }
    if chips.is_empty() {
        String::new()
    } else {
        format!(" [{}]", chips.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivitiesPresentation, ActivitiesRow, row_line_offsets};

    fn row(kind: &str, status: &str) -> ActivitiesRow {
        ActivitiesRow {
            id: format!("id-{kind}-{status}"),
            kind: kind.to_owned(),
            status: status.to_owned(),
            title: format!("{kind} {status}"),
            description: String::new(),
            command: None,
            session_id: None,
            started_at_ms: 0,
            finished_at_ms: None,
            exit_code: None,
            message: None,
            cancellable: true,
            dismissible: true,
        }
    }

    #[test]
    fn row_line_offsets_counts_section_headers() {
        let rows = [
            row("shell", "running"),
            row("task", "running"),
            row("shell", "succeeded"),
        ];
        let visible: Vec<&ActivitiesRow> = rows.iter().collect();
        // The Active section header sits at line 2 (blank + " Active"); the
        // Finished header adds another blank + header before its first row.
        assert_eq!(row_line_offsets(&visible), vec![2, 3, 6]);
    }

    #[test]
    fn reveal_selected_scrolls_down_and_resets_empty() {
        let rows = [
            row("shell", "running"),
            row("task", "running"),
            row("shell", "succeeded"),
        ];
        let visible: Vec<&ActivitiesRow> = rows.iter().collect();
        let offsets = row_line_offsets(&visible);

        // The last row is at line 6; a 2-line viewport must scroll it into view.
        let mut p1 = ActivitiesPresentation {
            selected: 2,
            ..ActivitiesPresentation::default()
        };
        p1.reveal_selected(&offsets, 2);
        assert_eq!(p1.scroll, 2);

        // An empty list resets both selection and scroll.
        let mut p2 = ActivitiesPresentation {
            selected: 5,
            scroll: 3,
            ..ActivitiesPresentation::default()
        };
        p2.reveal_selected(&[], 10);
        assert_eq!((p2.selected, p2.scroll), (0, 0));
    }
}
