use std::cmp::Ordering;

use agena::session::{UsageStats, UsageTotals};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{App, Frame, SurfaceMode};
use crate::app::{
    UsageDashboardControl, UsageDashboardSort, UsageDashboardState, UsageDashboardView,
    usage_dashboard_sort_label, usage_dashboard_view_label, usage_period_short_label,
    usage_sort_order, usage_sorted_sessions,
};

impl App {
    pub(in crate::app) fn render_usage_dashboard(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &UsageDashboardState,
        _surface: SurfaceMode,
    ) {
        let palette = agena_tui_components::theme::active_palette();
        let title = if state.loading {
            " Usage analytics  ·  refreshing… "
        } else {
            " Usage analytics "
        };
        let outer = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.muted));
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        if inner.width < 30 || inner.height < 10 {
            frame.render_widget(
                Paragraph::new("Terminal is too small for usage analytics")
                    .alignment(Alignment::Center),
                inner,
            );
            return;
        }

        let metric_height = if inner.width >= 96 { 5 } else { 4 };
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(metric_height),
                Constraint::Min(4),
                Constraint::Length(2),
            ])
            .split(inner);
        self.render_usage_header(frame, sections[0], state);
        self.render_usage_metrics(frame, sections[1], state.stats.as_ref());
        self.render_usage_content(frame, sections[2], state);
        self.render_usage_footer(frame, sections[3], state);
    }

    fn render_usage_header(&self, frame: &mut Frame, area: Rect, state: &UsageDashboardState) {
        let palette = agena_tui_components::theme::active_palette();
        let provider = state.provider_filter.as_deref().unwrap_or("All");
        let model = state.model_filter.as_deref().unwrap_or("All");
        let agents = if state.include_subagents {
            "included"
        } else {
            "excluded"
        };
        let control_labels = [
            format!("Period: {}", usage_period_short_label(state.period)),
            format!("View: {}", usage_dashboard_view_label(state.view)),
            format!("Provider: {provider}"),
            format!("Model: {model}"),
            format!("Subagents: {agents}"),
            format!("Sort: {}", usage_dashboard_sort_label(state.sort)),
            "Refresh".to_owned(),
        ];
        let controls = UsageDashboardControl::ALL
            .iter()
            .enumerate()
            .flat_map(|(index, _)| {
                let focused = state.controls_focused && state.selected_control == index;
                [
                    Span::styled(
                        format!("[ {} ]", control_labels[index]),
                        if focused {
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                        } else {
                            Style::default().fg(palette.muted)
                        },
                    ),
                    Span::raw(" "),
                ]
            })
            .collect::<Vec<_>>();
        let lines = vec![
            Line::from(controls),
            Line::from(Span::styled(
                if state.controls_focused {
                    "Enter changes the selected control"
                } else {
                    "Tab/Alt+Tab moves focus between content and dashboard controls"
                },
                Style::default().fg(palette.muted),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
    }

    fn render_usage_metrics(&self, frame: &mut Frame, area: Rect, stats: Option<&UsageStats>) {
        let Some(stats) = stats else {
            frame.render_widget(
                Paragraph::new("Loading usage statistics…")
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL)),
                area,
            );
            return;
        };
        let totals = &stats.totals;
        let values = [
            (
                "Total cost",
                format_cost(totals.total_cost_usd),
                format!(
                    "{} rec · {} est",
                    format_cost(totals.recorded_cost_usd),
                    format_cost(totals.estimated_cost_usd)
                ),
                agena_tui_components::theme::warning_color(),
            ),
            (
                "Tokens",
                format_tokens(totals.total_tokens),
                format!(
                    "{} in · {} out",
                    format_tokens(totals.input_tokens),
                    format_tokens(totals.output_tokens)
                ),
                agena_tui_components::theme::accent_color(),
            ),
            (
                "Runs",
                format_count(totals.runs),
                format!("{} avg", format_cost(stats.average_cost_per_run_usd)),
                agena_tui_components::theme::info_color(),
            ),
            (
                "Sessions",
                format_count(totals.sessions),
                format!("{} active days", stats.active_days),
                agena_tui_components::theme::special_color(),
            ),
            (
                "Cache hit",
                format_percent(totals.cache_hit_rate),
                format!(
                    "{} read · {} write",
                    format_tokens(totals.cache_read_tokens),
                    format_tokens(totals.cache_write_tokens)
                ),
                agena_tui_components::theme::success_color(),
            ),
        ];
        if area.width < 96 {
            let spans = values
                .iter()
                .enumerate()
                .flat_map(|(index, (label, value, _, color))| {
                    let separator = (index > 0).then(|| Span::raw("  │  "));
                    separator
                        .into_iter()
                        .chain([
                            Span::styled(
                                format!("{label} "),
                                Style::default().fg(agena_tui_components::theme::muted_color()),
                            ),
                            Span::styled(
                                value.clone(),
                                Style::default().fg(*color).add_modifier(Modifier::BOLD),
                            ),
                        ])
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(Line::from(spans))
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL)),
                area,
            );
            return;
        }
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 5); 5])
            .split(area);
        for ((label, value, detail, color), column) in values.iter().zip(columns.iter()) {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        *label,
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    )),
                    Line::from(Span::styled(
                        value.clone(),
                        Style::default().fg(*color).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        detail.clone(),
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    )),
                ])
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
                *column,
            );
        }
    }

    fn render_usage_content(&self, frame: &mut Frame, area: Rect, state: &UsageDashboardState) {
        if let Some(error) = state.error.as_deref() {
            frame.render_widget(
                Paragraph::new(format!(
                    "Unable to load usage statistics\n\n{error}\n\nSelect Refresh in the control bar and press Enter"
                ))
                .alignment(Alignment::Center)
                .style(Style::default().fg(agena_tui_components::theme::danger_color()))
                .block(Block::default().title(" Error ").borders(Borders::ALL)),
                area,
            );
            return;
        }
        let Some(stats) = state.stats.as_ref() else {
            frame.render_widget(
                Paragraph::new("Collecting message usage…")
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL)),
                area,
            );
            return;
        };
        match state.view {
            UsageDashboardView::Overview => self.render_usage_overview(frame, area, stats),
            UsageDashboardView::Daily => render_daily_table(frame, area, state, stats),
            UsageDashboardView::Providers => render_provider_table(frame, area, state, stats),
            UsageDashboardView::Models => render_model_table(frame, area, state, stats),
            UsageDashboardView::Sessions => render_session_table(frame, area, state),
        }
    }

    fn render_usage_overview(&self, frame: &mut Frame, area: Rect, stats: &UsageStats) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        render_daily_activity(frame, columns[0], stats);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(columns[1]);
        render_top_providers(frame, right[0], stats);
        render_top_models(frame, right[1], stats);
    }

    fn render_usage_footer(&self, frame: &mut Frame, area: Rect, state: &UsageDashboardState) {
        let mut hints =
            "Tab/Alt+Tab controls/content  ↑/↓ rows  Enter activate  Esc close".to_string();
        if state.view == UsageDashboardView::Sessions {
            hints.push_str("  Enter open session");
        }
        let range = state
            .stats
            .as_ref()
            .map(|stats| {
                let mut summary = format!(
                    "{} active days · {} avg/day · peak {} on {}",
                    stats.active_days,
                    format_cost(stats.average_cost_per_active_day_usd),
                    format_cost(stats.peak_cost_usd),
                    stats.peak_cost_date.as_deref().unwrap_or("—")
                );
                if stats.totals.unpriced_runs > 0 {
                    summary.push_str(
                        format!(" · ⚠ {} unpriced runs", stats.totals.unpriced_runs).as_str(),
                    );
                }
                summary
            })
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    hints,
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )),
                Line::from(Span::styled(
                    range,
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )),
            ]),
            area,
        );
    }
}

fn render_daily_activity(frame: &mut Frame, area: Rect, stats: &UsageStats) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let rows = stats
        .by_day
        .iter()
        .rev()
        .take(inner_height)
        .collect::<Vec<_>>();
    let max_cost = rows
        .iter()
        .map(|row| row.totals.total_cost_usd)
        .fold(0.0_f64, f64::max);
    let bar_width = area.width.saturating_sub(35).clamp(4, 24) as usize;
    let lines = rows
        .into_iter()
        .rev()
        .map(|day| {
            Line::from(vec![
                Span::styled(
                    format!("{} ", compact_date(day.date.as_str())),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ),
                Span::styled(
                    bar(day.totals.total_cost_usd, max_cost, bar_width),
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                ),
                Span::styled(
                    format!(" {:>9}", format_cost(day.totals.total_cost_usd)),
                    Style::default().fg(agena_tui_components::theme::warning_color()),
                ),
                Span::raw(format!(" {:>8}", format_tokens(day.totals.total_tokens))),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if lines.is_empty() {
            vec![Line::from("No daily activity")]
        } else {
            lines
        })
        .block(
            Block::default()
                .title(" Daily activity ")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_top_providers(frame: &mut Frame, area: Rect, stats: &UsageStats) {
    let rows = stats
        .by_provider
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|row| (&row.provider_id, &row.totals))
        .collect::<Vec<_>>();
    render_top_breakdown(frame, area, " By provider ", rows);
}

fn render_top_models(frame: &mut Frame, area: Rect, stats: &UsageStats) {
    let labels = stats
        .by_model
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|row| {
            (
                format!("{} · {}", row.provider_id, row.model_id),
                &row.totals,
            )
        })
        .collect::<Vec<_>>();
    let rows = labels
        .iter()
        .map(|(label, totals)| (label, *totals))
        .collect::<Vec<_>>();
    render_top_breakdown(frame, area, " By model ", rows);
}

fn render_top_breakdown<S: AsRef<str>>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    rows: Vec<(S, &UsageTotals)>,
) {
    let max_cost = rows
        .iter()
        .map(|(_, totals)| totals.total_cost_usd)
        .fold(0.0_f64, f64::max);
    let name_width = area.width.saturating_sub(23).max(8) as usize;
    let lines = rows
        .into_iter()
        .map(|(label, totals)| {
            let label = truncate(label.as_ref(), name_width);
            Line::from(vec![
                Span::styled(
                    format!("{label:<name_width$}"),
                    Style::default().fg(agena_tui_components::theme::info_color()),
                ),
                Span::styled(
                    bar(totals.total_cost_usd, max_cost, 6),
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                ),
                Span::styled(
                    format!(" {:>9}", format_cost(totals.total_cost_usd)),
                    Style::default().fg(agena_tui_components::theme::warning_color()),
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if lines.is_empty() {
            vec![Line::from("No usage")]
        } else {
            lines
        })
        .block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_daily_table(
    frame: &mut Frame,
    area: Rect,
    state: &UsageDashboardState,
    stats: &UsageStats,
) {
    let mut rows = stats.by_day.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| usage_sort_order(state.sort, &left.totals, &right.totals));
    let max = metric_max(state.sort, rows.iter().map(|row| &row.totals));
    let labels = rows.iter().map(|row| row.date.as_str()).collect::<Vec<_>>();
    render_usage_table(
        frame,
        area,
        " Daily breakdown ",
        labels.as_slice(),
        rows.iter()
            .map(|row| &row.totals)
            .collect::<Vec<_>>()
            .as_slice(),
        state,
        max,
        true,
    );
}

fn render_provider_table(
    frame: &mut Frame,
    area: Rect,
    state: &UsageDashboardState,
    stats: &UsageStats,
) {
    let mut rows = stats.by_provider.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| usage_sort_order(state.sort, &left.totals, &right.totals));
    let labels = rows
        .iter()
        .map(|row| row.provider_id.as_str())
        .collect::<Vec<_>>();
    let totals = rows.iter().map(|row| &row.totals).collect::<Vec<_>>();
    render_usage_table(
        frame,
        area,
        " Provider breakdown ",
        labels.as_slice(),
        totals.as_slice(),
        state,
        metric_max(state.sort, totals.iter().copied()),
        true,
    );
}

fn render_model_table(
    frame: &mut Frame,
    area: Rect,
    state: &UsageDashboardState,
    stats: &UsageStats,
) {
    let mut rows = stats.by_model.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| usage_sort_order(state.sort, &left.totals, &right.totals));
    let labels = rows
        .iter()
        .map(|row| format!("{} · {}", row.provider_id, row.model_id))
        .collect::<Vec<_>>();
    let totals = rows.iter().map(|row| &row.totals).collect::<Vec<_>>();
    render_usage_table(
        frame,
        area,
        " Model breakdown ",
        labels.as_slice(),
        totals.as_slice(),
        state,
        metric_max(state.sort, totals.iter().copied()),
        true,
    );
}

fn render_session_table(frame: &mut Frame, area: Rect, state: &UsageDashboardState) {
    let rows = usage_sorted_sessions(state);
    let labels = rows
        .iter()
        .map(|row| {
            format!(
                "#{} {}{}",
                row.session_id,
                if row.is_subagent { "↳ " } else { "" },
                row.title
            )
        })
        .collect::<Vec<_>>();
    let totals = rows.iter().map(|row| &row.totals).collect::<Vec<_>>();
    render_usage_table(
        frame,
        area,
        " Session breakdown ",
        labels.as_slice(),
        totals.as_slice(),
        state,
        metric_max(state.sort, totals.iter().copied()),
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_usage_table<S: AsRef<str>>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    labels: &[S],
    totals: &[&UsageTotals],
    state: &UsageDashboardState,
    max_metric: f64,
    selectable: bool,
) {
    let visible = area.height.saturating_sub(4) as usize;
    let start = if selectable {
        state
            .selected
            .saturating_sub(visible.saturating_sub(1) / 2)
            .min(labels.len().saturating_sub(visible))
    } else {
        0
    };
    let name_width = area.width.saturating_sub(59).max(10) as usize;
    let bar_width = area
        .width
        .saturating_sub((name_width + 43) as u16)
        .clamp(4, 18) as usize;
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{:<name_width$} {:<bar_width$} {:>10} {:>10} {:>7} {:>7}",
            "name", "", "cost", "tokens", "runs", "cache"
        ),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    ))];
    for (index, (label, total)) in labels
        .iter()
        .zip(totals.iter())
        .enumerate()
        .skip(start)
        .take(visible)
    {
        let selected = selectable && index == state.selected;
        let style = if selected {
            let palette = agena_tui_components::theme::active_palette();
            Style::default()
                .fg(palette.selection_fg)
                .bg(palette.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let marker = if selected { "›" } else { " " };
        let name = truncate(label.as_ref(), name_width.saturating_sub(2));
        lines.push(Line::from(Span::styled(
            format!(
                "{marker} {name:<name_width_inner$} {} {:>10} {:>10} {:>7} {:>7}",
                bar(metric_value(state.sort, total), max_metric, bar_width),
                format_cost(total.total_cost_usd),
                format_tokens(total.total_tokens),
                format_count(total.runs),
                format_percent(total.cache_hit_rate),
                name_width_inner = name_width.saturating_sub(2),
            ),
            style,
        )));
    }
    if labels.is_empty() {
        lines.push(Line::from("No usage for the selected filters"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn metric_value(sort: UsageDashboardSort, totals: &UsageTotals) -> f64 {
    match sort {
        UsageDashboardSort::Cost => totals.total_cost_usd,
        UsageDashboardSort::Tokens => totals.total_tokens as f64,
        UsageDashboardSort::Runs => totals.runs as f64,
    }
}

fn metric_max<'a>(sort: UsageDashboardSort, totals: impl Iterator<Item = &'a UsageTotals>) -> f64 {
    totals
        .map(|total| metric_value(sort, total))
        .fold(0.0_f64, f64::max)
}

fn bar(value: f64, max: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = if max <= 0.0 {
        0
    } else {
        ((value / max) * width as f64)
            .round()
            .clamp(0.0, width as f64) as usize
    };
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn format_cost(value: f64) -> String {
    if value >= 1_000.0 {
        format!("${value:.0}")
    } else if value >= 10.0 {
        format!("${value:.2}")
    } else {
        format!("${value:.4}")
    }
}

fn format_tokens(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}

fn format_percent(ratio: f64) -> String {
    format!("{:.1}%", ratio * 100.0)
}

fn compact_date(value: &str) -> &str {
    value.get(5..).unwrap_or(value)
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut output = value.chars().take(width - 1).collect::<String>();
    output.push('…');
    output
}

#[allow(dead_code)]
fn compare_f64_desc(left: f64, right: f64) -> Ordering {
    right.partial_cmp(&left).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::{bar, format_count, format_tokens, truncate};

    #[test]
    fn usage_formatters_are_compact_and_stable() {
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(format_tokens(1_250_000), "1.25M");
        assert_eq!(bar(5.0, 10.0, 4), "██░░");
        assert_eq!(truncate("abcdefgh", 5), "abcd…");
    }
}
