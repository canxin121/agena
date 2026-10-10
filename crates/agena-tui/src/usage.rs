//! Presentation vocabulary for the usage dashboard.
//!
//! Fetching usage data and navigating to a session remain application effects.
//! This module owns only the display tabs, sortable metrics, semantic controls,
//! and their stable labels.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::i18n::I18n;

/// Usage-dashboard data view selected by the terminal user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageDashboardView {
    #[default]
    Overview,
    Daily,
    Providers,
    Models,
    Sessions,
}

impl UsageDashboardView {
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Daily,
        Self::Providers,
        Self::Models,
        Self::Sessions,
    ];

    pub fn cycle(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|view| *view == self)
            .unwrap_or_default();
        let next = (index as isize + delta).rem_euclid(Self::ALL.len() as isize) as usize;
        Self::ALL[next]
    }
}

/// Metric used to order usage-dashboard rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageDashboardSort {
    #[default]
    Cost,
    Tokens,
    Requests,
}

impl UsageDashboardSort {
    pub fn next(self) -> Self {
        match self {
            Self::Cost => Self::Tokens,
            Self::Tokens => Self::Requests,
            Self::Requests => Self::Cost,
        }
    }
}

/// Semantic terminal control for the usage-dashboard reducer/effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageDashboardControl {
    Period,
    View,
    Provider,
    Model,
    Subagents,
    Sort,
    Refresh,
}

/// Pure usage-dashboard presentation state. Runtime query results and
/// application navigation effects intentionally stay outside this type.
#[derive(Debug, Clone, Default)]
pub struct UsageDashboardPresentation {
    pub view: UsageDashboardView,
    pub sort: UsageDashboardSort,
    pub include_subagents: bool,
    pub provider_filter: Option<String>,
    pub model_filter: Option<String>,
    pub selected: usize,
    pub scroll: usize,
}

/// Intent returned by the presentation reducer for an application adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageDashboardEffect {
    None,
    CyclePeriod,
    Reload,
}

/// Display-only usage totals. The application projects its Runtime query
/// result into this type before it reaches the terminal presentation layer.
#[derive(Debug, Clone, Default)]
pub struct UsageDashboardTotals {
    pub requests: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_write_5m_tokens: u64,
    pub cache_write_1h_tokens: u64,
    pub cache_read_tokens: u64,
    pub tool_use_tokens: u64,
    pub other_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_rate: f64,
    pub total_cost_usd: f64,
    pub recorded_cost_usd: f64,
    pub estimated_cost_usd: f64,
    pub unpriced_requests: u64,
    pub billable_unit_kinds: u64,
    pub unpriced_billable_units: f64,
}

/// An optional application navigation target carried by an otherwise
/// display-only usage row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDashboardSessionLink {
    pub session_id: i64,
    pub title: String,
}

/// A single usage breakdown row rendered by the dashboard.
#[derive(Debug, Clone)]
pub struct UsageDashboardRow {
    pub label: String,
    pub totals: UsageDashboardTotals,
    pub session: Option<UsageDashboardSessionLink>,
}

/// The complete display projection consumed by the usage-dashboard renderer.
/// It deliberately contains no Runtime, storage, or domain query types.
#[derive(Debug, Clone, Default)]
pub struct UsageDashboardData {
    pub totals: UsageDashboardTotals,
    pub active_days: u64,
    pub average_cost_per_request_usd: f64,
    pub average_cost_per_active_day_usd: f64,
    pub peak_cost_usd: f64,
    pub peak_cost_date: Option<String>,
    pub daily: Vec<UsageDashboardRow>,
    pub providers: Vec<UsageDashboardRow>,
    pub models: Vec<UsageDashboardRow>,
    pub sessions: Vec<UsageDashboardRow>,
}

impl UsageDashboardData {
    pub fn row_count(&self, view: UsageDashboardView) -> usize {
        self.rows_for_view(view).len()
    }

    pub fn sorted_rows(
        &self,
        view: UsageDashboardView,
        sort: UsageDashboardSort,
    ) -> Vec<&UsageDashboardRow> {
        let mut rows = self.rows_for_view(view).iter().collect::<Vec<_>>();
        rows.sort_by(|left, right| usage_dashboard_sort_order(sort, &left.totals, &right.totals));
        rows
    }

    pub fn selected_session(
        &self,
        presentation: &UsageDashboardPresentation,
    ) -> Option<&UsageDashboardSessionLink> {
        self.sorted_rows(UsageDashboardView::Sessions, presentation.sort)
            .get(presentation.selected)
            .and_then(|row| row.session.as_ref())
    }

    fn rows_for_view(&self, view: UsageDashboardView) -> &[UsageDashboardRow] {
        match view {
            UsageDashboardView::Overview => &[],
            UsageDashboardView::Daily => &self.daily,
            UsageDashboardView::Providers => &self.providers,
            UsageDashboardView::Models => &self.models,
            UsageDashboardView::Sessions => &self.sessions,
        }
    }
}

pub fn usage_dashboard_sort_order(
    sort: UsageDashboardSort,
    left: &UsageDashboardTotals,
    right: &UsageDashboardTotals,
) -> std::cmp::Ordering {
    match sort {
        UsageDashboardSort::Cost => right
            .total_cost_usd
            .partial_cmp(&left.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal),
        UsageDashboardSort::Tokens => right.total_tokens.cmp(&left.total_tokens),
        UsageDashboardSort::Requests => right.requests.cmp(&left.requests),
    }
    .then(right.requests.cmp(&left.requests))
}

impl UsageDashboardPresentation {
    pub fn new() -> Self {
        Self {
            include_subagents: true,
            ..Self::default()
        }
    }

    /// Reduces a semantic control into local presentation state and a query
    /// intent. `available_models` is `(provider_id, model_id)` so model
    /// filtering remains consistent with the active provider filter.
    pub fn activate(
        &mut self,
        control: UsageDashboardControl,
        available_providers: &[String],
        available_models: &[(String, String)],
    ) -> UsageDashboardEffect {
        let effect = match control {
            UsageDashboardControl::Period => UsageDashboardEffect::CyclePeriod,
            UsageDashboardControl::View => {
                self.view = self.view.cycle(1);
                UsageDashboardEffect::None
            }
            UsageDashboardControl::Provider => {
                cycle_provider_filter(self, available_providers, 1);
                UsageDashboardEffect::Reload
            }
            UsageDashboardControl::Model => {
                cycle_model_filter(self, available_models, 1);
                UsageDashboardEffect::Reload
            }
            UsageDashboardControl::Subagents => {
                self.include_subagents = !self.include_subagents;
                UsageDashboardEffect::Reload
            }
            UsageDashboardControl::Sort => {
                self.sort = self.sort.next();
                UsageDashboardEffect::None
            }
            UsageDashboardControl::Refresh => UsageDashboardEffect::Reload,
        };
        self.reset_selection();
        effect
    }

    pub fn move_selection(&mut self, row_count: usize, delta: isize) {
        if row_count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, row_count as isize - 1) as usize;
    }

    pub fn clamp_selection(&mut self, row_count: usize) {
        self.selected = self.selected.min(row_count.saturating_sub(1));
    }

    pub fn reset_selection(&mut self) {
        self.selected = 0;
        self.scroll = 0;
    }
}

fn cycle_provider_filter(
    presentation: &mut UsageDashboardPresentation,
    available_providers: &[String],
    delta: isize,
) {
    let mut choices = Vec::with_capacity(available_providers.len() + 1);
    choices.push(None);
    choices.extend(available_providers.iter().cloned().map(Some));
    let index = choices
        .iter()
        .position(|choice| choice == &presentation.provider_filter)
        .unwrap_or_default();
    let next = (index as isize + delta).rem_euclid(choices.len() as isize) as usize;
    presentation.provider_filter = choices[next].clone();
    presentation.model_filter = None;
}

fn cycle_model_filter(
    presentation: &mut UsageDashboardPresentation,
    available_models: &[(String, String)],
    delta: isize,
) {
    let mut choices = Vec::with_capacity(available_models.len() + 1);
    choices.push(None);
    for (provider, model) in available_models {
        if presentation
            .provider_filter
            .as_ref()
            .is_some_and(|selected| selected != provider)
        {
            continue;
        }
        let model = Some(model.clone());
        if !choices.contains(&model) {
            choices.push(model);
        }
    }
    let index = choices
        .iter()
        .position(|choice| choice == &presentation.model_filter)
        .unwrap_or_default();
    let next = (index as isize + delta).rem_euclid(choices.len() as isize) as usize;
    presentation.model_filter = choices[next].clone();
}

pub fn usage_dashboard_view_label(i18n: &I18n, view: UsageDashboardView) -> String {
    i18n.text(match view {
        UsageDashboardView::Overview => "usage-view-overview",
        UsageDashboardView::Daily => "usage-view-daily",
        UsageDashboardView::Providers => "usage-view-providers",
        UsageDashboardView::Models => "usage-view-models",
        UsageDashboardView::Sessions => "usage-view-sessions",
    })
}

pub fn usage_dashboard_sort_label(i18n: &I18n, sort: UsageDashboardSort) -> String {
    i18n.text(match sort {
        UsageDashboardSort::Cost => "usage-sort-cost",
        UsageDashboardSort::Tokens => "usage-sort-tokens",
        UsageDashboardSort::Requests => "usage-sort-requests",
    })
}

/// Renders the complete usage-dashboard presentation from the display-only
/// projection supplied by the application adapter.
#[allow(clippy::too_many_arguments)]
pub fn render_usage_dashboard(
    frame: &mut Frame<'_>,
    area: Rect,
    period_label: &str,
    loading: bool,
    error: Option<&str>,
    presentation: &UsageDashboardPresentation,
    data: Option<&UsageDashboardData>,
    i18n: &I18n,
) {
    let title = i18n.text(if loading {
        "usage-title-refreshing"
    } else {
        "usage-title"
    });
    let inner = agena_tui_components::render_framed_surface(
        frame,
        area,
        agena_tui_components::SurfaceMode::Route,
        &agena_tui_components::FramedSurfaceSpec {
            title: title.into(),
            target_width: area.width,
            target_height: area.height,
        },
    )
    .inner;
    if inner.width < 30 || inner.height < 10 {
        frame.render_widget(
            Paragraph::new(i18n.text("usage-to-small")).alignment(Alignment::Center),
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
    render_usage_header(frame, sections[0], period_label, presentation, i18n);
    render_usage_metrics(frame, sections[1], data, i18n);
    render_usage_content(frame, sections[2], presentation, data, error, i18n);
    render_usage_footer(frame, sections[3], presentation, data, i18n);
}

fn render_usage_header(
    frame: &mut Frame<'_>,
    area: Rect,
    period_label: &str,
    presentation: &UsageDashboardPresentation,
    i18n: &I18n,
) {
    let palette = agena_tui_components::theme::active_palette();
    let all = i18n.text("usage-filter-all");
    let provider = presentation
        .provider_filter
        .clone()
        .unwrap_or_else(|| all.clone());
    let model = presentation.model_filter.clone().unwrap_or(all);
    let agents = i18n.text(if presentation.include_subagents {
        "usage-filter-subagents-included"
    } else {
        "usage-filter-subagents-excluded"
    });
    let shortcut = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::BOLD);
    let value = Style::default().fg(palette.muted);
    let controls = vec![
        Span::styled("Ctrl+P", shortcut),
        Span::styled(
            format!(
                " {}",
                i18n.text_args(
                    "usage-filter-period",
                    &crate::fl_args!("period" => period_label)
                )
            ),
            value,
        ),
        Span::raw("  ·  "),
        Span::styled("Ctrl+B", shortcut),
        Span::styled(
            format!(
                " {}",
                i18n.text_args(
                    "usage-filter-view",
                    &crate::fl_args!("view" => usage_dashboard_view_label(i18n, presentation.view))
                )
            ),
            value,
        ),
        Span::raw("  ·  "),
        Span::styled("Ctrl+O", shortcut),
        Span::styled(
            format!(
                " {}",
                i18n.text_args(
                    "usage-filter-provider",
                    &crate::fl_args!("provider" => provider)
                )
            ),
            value,
        ),
        Span::raw("  ·  "),
        Span::styled("Ctrl+L", shortcut),
        Span::styled(
            format!(
                " {}",
                i18n.text_args("usage-filter-model", &crate::fl_args!("model" => model))
            ),
            value,
        ),
        Span::raw("  ·  "),
        Span::styled("Ctrl+A", shortcut),
        Span::styled(
            format!(
                " {}",
                i18n.text_args(
                    "usage-filter-subagents",
                    &crate::fl_args!("state" => agents)
                )
            ),
            value,
        ),
        Span::raw("  ·  "),
        Span::styled("Ctrl+S", shortcut),
        Span::styled(
            format!(
                " {}",
                i18n.text_args(
                    "usage-filter-sort",
                    &crate::fl_args!("sort" => usage_dashboard_sort_label(i18n, presentation.sort))
                )
            ),
            value,
        ),
        Span::raw("  ·  "),
        Span::styled("Ctrl+R", shortcut),
        Span::styled(format!(" {}", i18n.text("usage-filter-refresh")), value),
    ];
    let controls = agena_tui_components::line_plain_text(&Line::from(controls));
    agena_tui_components::render_shortcut_footer(
        frame,
        area,
        &format!("{controls} · {}", i18n.text("usage-footer")),
    );
}

fn render_usage_metrics(
    frame: &mut Frame<'_>,
    area: Rect,
    data: Option<&UsageDashboardData>,
    i18n: &I18n,
) {
    let Some(data) = data else {
        frame.render_widget(
            Paragraph::new(i18n.text("usage-loading"))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let totals = &data.totals;
    let values = [
        (
            i18n.text("usage-total-cost"),
            format_cost(totals.total_cost_usd),
            i18n.text_args(
                "usage-cost-line",
                &crate::fl_args!(
                    "recorded" => format_cost(totals.recorded_cost_usd),
                    "estimated" => format_cost(totals.estimated_cost_usd)
                ),
            ),
            agena_tui_components::theme::warning_color(),
        ),
        (
            i18n.text("usage-tokens"),
            format_tokens(totals.total_tokens),
            i18n.text_args(
                "usage-tokens-line",
                &crate::fl_args!(
                    "input" => format_tokens(totals.input_tokens),
                    "output" => format_tokens(totals.output_tokens),
                    "reasoning" => format_tokens(totals.reasoning_tokens)
                ),
            ),
            agena_tui_components::theme::accent_color(),
        ),
        (
            i18n.text("usage-requests"),
            format_count(totals.requests),
            i18n.text_args(
                "usage-average-cost",
                &crate::fl_args!("cost" => format_cost(data.average_cost_per_request_usd)),
            ),
            agena_tui_components::theme::info_color(),
        ),
        (
            i18n.text("usage-sessions"),
            format_count(totals.sessions),
            i18n.text_args(
                "usage-active-days",
                &crate::fl_args!("count" => data.active_days),
            ),
            agena_tui_components::theme::special_color(),
        ),
        (
            i18n.text("usage-cache-hit"),
            format_percent(totals.cache_hit_rate),
            i18n.text_args(
                "usage-cache-line",
                &crate::fl_args!(
                    "read" => format_tokens(totals.cache_read_tokens),
                    "write" => format_tokens(totals.cache_write_tokens),
                    "write_5m" => format_tokens(totals.cache_write_5m_tokens),
                    "write_1h" => format_tokens(totals.cache_write_1h_tokens)
                ),
            ),
            agena_tui_components::theme::success_color(),
        ),
    ];
    if area.width < 96 {
        let spans = values
            .iter()
            .enumerate()
            .flat_map(|(index, (label, value, _, color))| {
                (index > 0)
                    .then(|| Span::raw("  │  "))
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
                    label.clone(),
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

fn render_usage_content(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &UsageDashboardPresentation,
    data: Option<&UsageDashboardData>,
    error: Option<&str>,
    i18n: &I18n,
) {
    if let Some(error) = error {
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n\n{error}\n\n{}",
                i18n.text("usage-error-body"),
                i18n.text("usage-error-hint")
            ))
            .alignment(Alignment::Center)
            .style(Style::default().fg(agena_tui_components::theme::danger_color()))
            .block(
                Block::default()
                    .title(format!(" {} ", i18n.text("usage-error-badge")))
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    }
    let Some(data) = data else {
        frame.render_widget(
            Paragraph::new(i18n.text("usage-collecting"))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    if presentation.view == UsageDashboardView::Overview {
        render_usage_overview(frame, area, data, i18n);
    } else {
        let title = match presentation.view {
            UsageDashboardView::Daily => i18n.text("usage-breakdown-daily"),
            UsageDashboardView::Providers => i18n.text("usage-breakdown-provider"),
            UsageDashboardView::Models => i18n.text("usage-breakdown-model"),
            UsageDashboardView::Sessions => i18n.text("usage-breakdown-session"),
            UsageDashboardView::Overview => unreachable!(),
        };
        render_usage_table(
            frame,
            area,
            &format!(" {title} "),
            data.sorted_rows(presentation.view, presentation.sort)
                .as_slice(),
            presentation,
            i18n,
        );
    }
}

fn render_usage_overview(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &UsageDashboardData,
    i18n: &I18n,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    render_overview_rows(
        frame,
        columns[0],
        &format!(" {} ", i18n.text("usage-daily-activity")),
        &data.daily,
        true,
        i18n,
    );
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(columns[1]);
    render_overview_rows(
        frame,
        right[0],
        &format!(" {} ", i18n.text("usage-by-provider")),
        &data.providers,
        false,
        i18n,
    );
    render_overview_rows(
        frame,
        right[1],
        &format!(" {} ", i18n.text("usage-by-model")),
        &data.models,
        false,
        i18n,
    );
}

fn render_overview_rows(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    rows: &[UsageDashboardRow],
    daily: bool,
    i18n: &I18n,
) {
    let visible = if daily {
        rows.iter()
            .rev()
            .take(area.height.saturating_sub(2) as usize)
            .collect::<Vec<_>>()
    } else {
        rows.iter()
            .take(area.height.saturating_sub(2) as usize)
            .collect::<Vec<_>>()
    };
    let max_cost = visible
        .iter()
        .map(|row| row.totals.total_cost_usd)
        .fold(0.0_f64, f64::max);
    let bar_width = if daily {
        area.width.saturating_sub(35).clamp(4, 24) as usize
    } else {
        6
    };
    let name_width = area.width.saturating_sub(23).max(8) as usize;
    let rows = if daily {
        visible.into_iter().rev().collect::<Vec<_>>()
    } else {
        visible
    };
    let lines = rows
        .into_iter()
        .map(|row| {
            let label = if daily {
                compact_date(&row.label).to_owned()
            } else {
                truncate(&row.label, name_width)
            };
            Line::from(vec![
                Span::styled(
                    if daily {
                        format!("{label} ")
                    } else {
                        format!("{label:<name_width$}")
                    },
                    Style::default().fg(agena_tui_components::theme::info_color()),
                ),
                Span::styled(
                    bar(row.totals.total_cost_usd, max_cost, bar_width),
                    Style::default().fg(agena_tui_components::theme::accent_color()),
                ),
                Span::styled(
                    format!(" {:>9}", format_cost(row.totals.total_cost_usd)),
                    Style::default().fg(agena_tui_components::theme::warning_color()),
                ),
                Span::raw(if daily {
                    format!(" {:>8}", format_tokens(row.totals.total_tokens))
                } else {
                    String::new()
                }),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if lines.is_empty() {
            vec![Line::from(i18n.text("usage-empty"))]
        } else {
            lines
        })
        .block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_usage_table(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    rows: &[&UsageDashboardRow],
    presentation: &UsageDashboardPresentation,
    i18n: &I18n,
) {
    let visible = area.height.saturating_sub(4) as usize;
    let start = presentation
        .selected
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(rows.len().saturating_sub(visible));
    let name_width = area.width.saturating_sub(59).max(10) as usize;
    let bar_width = area
        .width
        .saturating_sub((name_width + 43) as u16)
        .clamp(4, 18) as usize;
    let max_metric = rows
        .iter()
        .map(|row| metric_value(presentation.sort, &row.totals))
        .fold(0.0_f64, f64::max);
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{:<name_width$} {:<bar_width$} {:>10} {:>10} {:>7} {:>7}",
            i18n.text("usage-column-name"),
            "",
            i18n.text("usage-column-cost"),
            i18n.text("usage-column-tokens"),
            i18n.text("usage-column-requests"),
            i18n.text("usage-column-cache")
        ),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    ))];
    for (index, row) in rows.iter().enumerate().skip(start).take(visible) {
        agena_tui_components::pointer::register(
            Rect::new(
                area.x.saturating_add(1),
                area.y.saturating_add(2 + (index - start) as u16),
                area.width.saturating_sub(2),
                1,
            ),
            Some(agena_tui_components::pointer::PointerAction::List {
                panel: 0,
                index,
                selected: presentation.selected,
            }),
            None,
        );
        let selected = index == presentation.selected;
        let style = if selected {
            let palette = agena_tui_components::theme::active_palette();
            Style::default()
                .fg(palette.selection_fg)
                .bg(palette.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let name = truncate(&row.label, name_width.saturating_sub(2));
        lines.push(Line::from(Span::styled(
            format!(
                "{} {name:<width$} {} {:>10} {:>10} {:>7} {:>7}",
                if selected { "›" } else { " " },
                bar(
                    metric_value(presentation.sort, &row.totals),
                    max_metric,
                    bar_width
                ),
                format_cost(row.totals.total_cost_usd),
                format_tokens(row.totals.total_tokens),
                format_count(row.totals.requests),
                format_percent(row.totals.cache_hit_rate),
                width = name_width.saturating_sub(2)
            ),
            style,
        )));
    }
    if rows.is_empty() {
        lines.push(Line::from(i18n.text("usage-empty-filtered")));
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_usage_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &UsageDashboardPresentation,
    data: Option<&UsageDashboardData>,
    i18n: &I18n,
) {
    let mut hints = i18n.text("usage-footer-rows");
    if presentation.view == UsageDashboardView::Sessions {
        hints.push_str(&format!("  {}", i18n.text("usage-footer-open-session")));
    }
    let range = data
        .map(|data| {
            let mut summary = i18n.text_args(
                "usage-days-summary",
                &crate::fl_args!(
                    "days" => data.active_days,
                    "average" => format_cost(data.average_cost_per_active_day_usd),
                    "peak" => format_cost(data.peak_cost_usd),
                    "date" => data.peak_cost_date.as_deref().unwrap_or("—")
                ),
            );
            if data.totals.unpriced_requests > 0 {
                summary.push_str(&format!(
                    " · {}",
                    i18n.text_args(
                        "usage-unpriced-requests",
                        &crate::fl_args!("count" => data.totals.unpriced_requests)
                    )
                ));
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

fn metric_value(sort: UsageDashboardSort, totals: &UsageDashboardTotals) -> f64 {
    match sort {
        UsageDashboardSort::Cost => totals.total_cost_usd,
        UsageDashboardSort::Tokens => totals.total_tokens as f64,
        UsageDashboardSort::Requests => totals.requests as f64,
    }
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
        return value.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut output = value.chars().take(width - 1).collect::<String>();
    output.push('…');
    output
}

#[cfg(test)]
mod tests {
    use super::{
        UsageDashboardControl, UsageDashboardData, UsageDashboardEffect,
        UsageDashboardPresentation, UsageDashboardRow, UsageDashboardSessionLink,
        UsageDashboardSort, UsageDashboardTotals, UsageDashboardView, usage_dashboard_sort_label,
        usage_dashboard_view_label,
    };
    use crate::i18n::I18n;

    #[test]
    fn views_cycle_in_presentation_order() {
        assert_eq!(
            UsageDashboardView::Overview.cycle(-1),
            UsageDashboardView::Sessions
        );
        assert_eq!(
            UsageDashboardView::Sessions.cycle(1),
            UsageDashboardView::Overview
        );
    }

    #[test]
    fn sort_cycles_and_labels_are_stable() {
        let i18n = I18n::english();
        assert_eq!(UsageDashboardSort::Cost.next(), UsageDashboardSort::Tokens);
        assert_eq!(
            UsageDashboardSort::Requests.next(),
            UsageDashboardSort::Cost
        );
        assert_eq!(
            usage_dashboard_view_label(&i18n, UsageDashboardView::Providers),
            i18n.text("usage-view-providers")
        );
        assert_eq!(
            usage_dashboard_sort_label(&i18n, UsageDashboardSort::Tokens),
            i18n.text("usage-sort-tokens")
        );
    }

    #[test]
    fn reducer_resets_selection_and_returns_query_intent() {
        let mut presentation = UsageDashboardPresentation {
            selected: 4,
            scroll: 2,
            ..UsageDashboardPresentation::new()
        };
        assert_eq!(
            presentation.activate(
                UsageDashboardControl::Provider,
                &["openai".to_owned()],
                &[("openai".to_owned(), "gpt-5".to_owned())],
            ),
            UsageDashboardEffect::Reload
        );
        assert_eq!(presentation.provider_filter.as_deref(), Some("openai"));
        assert_eq!(presentation.selected, 0);
        assert_eq!(presentation.scroll, 0);
        assert_eq!(
            presentation.activate(UsageDashboardControl::Period, &[], &[]),
            UsageDashboardEffect::CyclePeriod
        );
    }

    #[test]
    fn display_rows_sort_and_resolve_the_selected_session_without_domain_values() {
        let data = UsageDashboardData {
            sessions: vec![
                UsageDashboardRow {
                    label: "#1 first".to_owned(),
                    totals: UsageDashboardTotals {
                        total_cost_usd: 1.0,
                        ..UsageDashboardTotals::default()
                    },
                    session: Some(UsageDashboardSessionLink {
                        session_id: 1,
                        title: "first".to_owned(),
                    }),
                },
                UsageDashboardRow {
                    label: "#2 second".to_owned(),
                    totals: UsageDashboardTotals {
                        total_cost_usd: 2.0,
                        ..UsageDashboardTotals::default()
                    },
                    session: Some(UsageDashboardSessionLink {
                        session_id: 2,
                        title: "second".to_owned(),
                    }),
                },
            ],
            ..UsageDashboardData::default()
        };
        let presentation = UsageDashboardPresentation::new();
        assert_eq!(data.row_count(UsageDashboardView::Sessions), 2);
        assert_eq!(
            data.selected_session(&presentation)
                .map(|link| link.session_id),
            Some(2)
        );
    }
}
