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
    Control { id: String, action: String },
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
    pub source_part_id: Option<i64>,
    pub next_event_at_ms: Option<i64>,
    pub controls: Vec<String>,
    pub cancellable: bool,
    pub dismissible: bool,
}

impl ActivitiesRow {
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "running" | "paused")
    }

    pub fn running_seconds(&self, now_ms: i64) -> Option<i64> {
        let start = self.started_at_ms;
        let end = self.finished_at_ms.unwrap_or(now_ms);
        (end >= start).then_some((end - start) / 1000)
    }

    /// Whether an opened detail pane should keep following this member's log
    /// cursor. Paused schedules remain managed/active but produce no stream.
    pub fn logs_are_live(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "running") && self.kind != "cron"
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
            if !filter.show_finished && !row.is_active() {
                return false;
            }
            if let Some(kind) = filter.kind_filter.as_deref()
                && row.kind != kind
            {
                return false;
            }
            if let Some(status) = filter.status_filter.as_deref()
                && row.status != status
            {
                return false;
            }
            true
        })
        .collect()
}

/// Resolve selection through the filtered display projection. The selected
/// index is never an index into the unfiltered backing vector.
pub fn selected_row<'a>(
    rows: &'a [ActivitiesRow],
    presentation: &ActivitiesPresentation,
) -> Option<&'a ActivitiesRow> {
    visible_rows(rows, presentation)
        .get(presentation.selected)
        .copied()
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

    /// Cycle the kind filter across every unified background member kind.
    pub fn cycle_kind_filter(&mut self) {
        self.kind_filter = match self.kind_filter.as_deref() {
            None => Some("shell".to_owned()),
            Some("shell") => Some("monitor".to_owned()),
            Some("monitor") => Some("task".to_owned()),
            Some("task") => Some("cron".to_owned()),
            Some("cron") => Some("runtime".to_owned()),
            Some("runtime") => Some("browser".to_owned()),
            _ => None,
        };
        self.selected = 0;
        self.scroll = 0;
    }

    /// Cycle across every lifecycle state exposed by the activity API.
    pub fn cycle_status_filter(&mut self) {
        self.status_filter = match self.status_filter.as_deref() {
            None => Some("running".to_owned()),
            Some("running") => Some("pending".to_owned()),
            Some("pending") => Some("paused".to_owned()),
            Some("paused") => Some("failed".to_owned()),
            Some("failed") => Some("succeeded".to_owned()),
            Some("succeeded") => Some("cancelled".to_owned()),
            Some("cancelled") => Some("stopped".to_owned()),
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
        "monitor" => "Monitor",
        "task" => "Task",
        "cron" => "Cron",
        "runtime" => "Runtime",
        "browser" => "Browser",
        _ => "Activity",
    }
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "pending" => "pending",
        "running" => "running",
        "paused" => "paused",
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
        "monitor" => "\u{224B}", // ≋
        "task" => "\u{25C8}",    // ◈
        "cron" => "\u{25F7}",    // ◷
        "runtime" => "\u{21BB}", // ↻
        "browser" => "\u{25C9}", // ◉
        _ => "\u{2022}",         // •
    }
}

fn status_style(status: &str) -> Style {
    match status {
        "running" | "pending" => Style::default().fg(accent_color()),
        "paused" => Style::default().fg(warning_color()),
        "succeeded" => Style::default().fg(success_color()),
        "failed" => Style::default().fg(danger_color()),
        "cancelled" | "stopped" => Style::default().fg(warning_color()),
        _ => muted_style(),
    }
}

fn kind_style(kind: &str) -> Style {
    match kind {
        "shell" => Style::default().fg(info_color()),
        "monitor" => Style::default().fg(accent_color()),
        "task" => Style::default().fg(special_color()),
        "cron" => Style::default().fg(warning_color()),
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

/// Responsive list/detail geometry shared by rendering and keyboard paging.
/// The list owns the full surface while detail is closed; narrow terminals
/// stack detail vertically so neither pane becomes an unreadable sliver.
pub fn activities_pane_areas(area: Rect, detail: bool) -> (Rect, Option<Rect>) {
    if !detail {
        return (area, None);
    }
    let direction = if area.width >= 100 {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let panes = Layout::default()
        .direction(direction)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    (panes[0], Some(panes[1]))
}

/// Ephemeral runtime values used while drawing the panel. Keeping these
/// separate from [`ActivitiesPresentation`] avoids mixing application-owned
/// request state into the pure navigation model.
#[derive(Debug, Clone, Copy)]
pub struct ActivitiesPanelContext<'a> {
    pub loading: bool,
    pub error: Option<&'a str>,
    pub log_tail: Option<&'a ActivitiesLogTail>,
    pub log_error: Option<&'a str>,
    pub now_ms: i64,
}

/// Render the complete activities panel into `area`.
pub fn render_activities_panel(
    frame: &mut Frame,
    area: Rect,
    presentation: &ActivitiesPresentation,
    rows: &[ActivitiesRow],
    context: ActivitiesPanelContext<'_>,
) {
    let (list_area, detail_area) = activities_pane_areas(area, presentation.detail);

    render_list_pane(
        frame,
        list_area,
        presentation,
        rows,
        context.loading,
        context.error,
        context.now_ms,
    );
    if let Some(detail_area) = detail_area {
        render_detail_pane(
            frame,
            detail_area,
            presentation,
            rows,
            context.log_tail,
            context.log_error,
        );
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
        .title(Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )))
        .title_bottom(Line::from(action_bar(
            presentation,
            &visible,
            active_count,
            finished_count,
            area.width,
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
        let mut last_section: Option<&'static str> = None;
        for (index, row) in visible.iter().enumerate() {
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
    // Rows intentionally stay single-line. Wrapping would make selection and
    // paging disagree with rendered row heights on narrow terminals.
    frame.render_widget(Paragraph::new(visible_lines), inner);
}

fn action_bar(
    presentation: &ActivitiesPresentation,
    visible: &[&ActivitiesRow],
    active_count: usize,
    finished_count: usize,
    width: u16,
) -> String {
    // Keep close/selection at the front so tiny terminals clip optional
    // controls rather than the only reliable way out of the overlay.
    let mut actions = vec![
        "Esc close".to_owned(),
        "↑↓ move".to_owned(),
        "↵ detail".to_owned(),
    ];
    if let Some(row) = visible.get(presentation.selected) {
        if let Some(control) = row
            .controls
            .iter()
            .find(|control| matches!(control.as_str(), "stop" | "pause" | "resume"))
        {
            actions.push(format!("s {control}"));
        }
        if let Some(control) = row
            .controls
            .iter()
            .find(|control| matches!(control.as_str(), "delete" | "dismiss"))
        {
            actions.push(format!("d {control}"));
        }
    }
    if finished_count > 0 {
        actions.push("x clear".to_owned());
    }
    if width >= 90 {
        actions.push("f/k/t filters".to_owned());
    }
    actions.push("r refresh".to_owned());
    format!(
        " {active_count} active · {finished_count} finished | {} ",
        actions.join("  ")
    )
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
    log_error: Option<&str>,
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

    let Some(selected) = selected_row(rows, presentation) else {
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
    if let Some(source_part_id) = selected.source_part_id {
        lines.push(Line::from(Span::styled(
            format!(" source part #{source_part_id}"),
            muted_style(),
        )));
    }
    if let Some(next_event_at_ms) = selected.next_event_at_ms {
        lines.push(Line::from(Span::styled(
            format!(" next wake {next_event_at_ms}"),
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

    if let Some(error) = log_error {
        lines.push(Line::from(Span::styled(
            format!(" ✗ {error}"),
            Style::default().fg(danger_color()),
        )));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    match log_tail {
        Some(tail) if !tail.lines.is_empty() => {
            lines.push(Line::from(Span::styled(
                format!(" — output · seq {} — ", tail.last_seq),
                Style::default()
                    .fg(warning_color())
                    .add_modifier(Modifier::BOLD),
            )));
            let marker_lines = usize::from(tail.has_more || tail.dropped_lines > 0);
            let available = (inner.height as usize)
                .saturating_sub(lines.len())
                .saturating_sub(marker_lines);
            let start = tail.lines.len().saturating_sub(available);
            for line in tail.lines.iter().skip(start) {
                lines.push(Line::from(Span::styled(line.clone(), Style::default())));
            }
            if tail.has_more || tail.dropped_lines > 0 {
                lines.push(Line::from(Span::styled(
                    format!(" … {} older lines not shown", tail.dropped_lines),
                    muted_style(),
                )));
            }
        }
        Some(_) => {
            let message = if selected.is_active() {
                " Waiting for output…"
            } else {
                " No output recorded."
            };
            lines.push(Line::from(Span::styled(message, muted_style())));
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
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    use super::{
        ActivitiesLogTail, ActivitiesPanelContext, ActivitiesPresentation, ActivitiesRow,
        activities_pane_areas, render_activities_panel, row_line_offsets, selected_row,
    };

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
            source_part_id: None,
            next_event_at_ms: None,
            controls: if matches!(status, "running" | "pending") {
                vec!["stop".to_owned()]
            } else {
                vec!["dismiss".to_owned()]
            },
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

    #[test]
    fn selected_row_resolves_through_filters() {
        let rows = [
            row("shell", "running"),
            row("browser", "running"),
            row("task", "failed"),
        ];
        let presentation = ActivitiesPresentation {
            selected: 0,
            kind_filter: Some("browser".to_owned()),
            ..ActivitiesPresentation::default()
        };
        assert_eq!(
            selected_row(&rows, &presentation).map(|row| row.id.as_str()),
            Some("id-browser-running")
        );
    }

    #[test]
    fn status_filter_cycles_cancelled_and_stopped() {
        let mut presentation = ActivitiesPresentation::default();
        let mut statuses = Vec::new();
        for _ in 0..8 {
            presentation.cycle_status_filter();
            statuses.push(presentation.status_filter.clone());
        }
        assert_eq!(
            statuses,
            vec![
                Some("running".to_owned()),
                Some("pending".to_owned()),
                Some("paused".to_owned()),
                Some("failed".to_owned()),
                Some("succeeded".to_owned()),
                Some("cancelled".to_owned()),
                Some("stopped".to_owned()),
                None,
            ]
        );
    }

    #[test]
    fn detail_layout_is_full_width_closed_and_responsive_when_open() {
        let area = Rect::new(0, 0, 120, 40);
        assert_eq!(activities_pane_areas(area, false), (area, None));

        let (wide_list, wide_detail) = activities_pane_areas(area, true);
        let wide_detail = wide_detail.expect("wide detail pane");
        assert_eq!(wide_list.y, wide_detail.y);
        assert!(wide_list.x < wide_detail.x);

        let (narrow_list, narrow_detail) = activities_pane_areas(Rect::new(0, 0, 80, 40), true);
        let narrow_detail = narrow_detail.expect("narrow detail pane");
        assert_eq!(narrow_list.x, narrow_detail.x);
        assert!(narrow_list.y < narrow_detail.y);
    }

    fn render_to_rows(
        width: u16,
        height: u16,
        presentation: &ActivitiesPresentation,
        rows: &[ActivitiesRow],
        log_tail: Option<&ActivitiesLogTail>,
    ) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render_activities_panel(
                    frame,
                    frame.area(),
                    presentation,
                    rows,
                    ActivitiesPanelContext {
                        loading: false,
                        error: None,
                        log_tail,
                        log_error: None,
                        now_ms: 10_000,
                    },
                );
            })
            .expect("render activities");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn narrow_footer_exposes_the_real_close_key() {
        let rows = [row("shell", "running")];
        let rendered =
            render_to_rows(80, 20, &ActivitiesPresentation::default(), &rows, None).join("\n");
        assert!(rendered.contains("Esc close"), "{rendered}");
        assert!(!rendered.contains("q close"), "{rendered}");
    }

    #[test]
    fn detail_log_tail_keeps_chronological_order() {
        let rows = [row("shell", "running")];
        let presentation = ActivitiesPresentation {
            detail: true,
            ..ActivitiesPresentation::default()
        };
        let tail = ActivitiesLogTail {
            lines: vec!["first output".to_owned(), "second output".to_owned()],
            last_seq: 2,
            has_more: false,
            dropped_lines: 0,
        };
        let rendered = render_to_rows(120, 30, &presentation, &rows, Some(&tail)).join("\n");
        let first = rendered.find("first output").expect("first log line");
        let second = rendered.find("second output").expect("second log line");
        assert!(first < second, "{rendered}");
    }
}
