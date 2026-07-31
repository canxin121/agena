use agena_domain::{UsagePeriod, UsageStats, UsageStatsQuery, UsageTotals};

use super::{App, AppMessage, KeyEvent, Route, UsageDashboardState, Utc};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use agena_tui::usage::{
    UsageDashboardControl, UsageDashboardData, UsageDashboardEffect, UsageDashboardRow,
    UsageDashboardSessionLink, UsageDashboardTotals, UsageDashboardView,
};

pub(super) const USAGE_PERIODS: [UsagePeriod; 9] = [
    UsagePeriod::Today,
    UsagePeriod::Yesterday,
    UsagePeriod::Last7Days,
    UsagePeriod::Last14Days,
    UsagePeriod::Last30Days,
    UsagePeriod::Last90Days,
    UsagePeriod::MonthToDate,
    UsagePeriod::YearToDate,
    UsagePeriod::AllTime,
];

impl App {
    pub(crate) fn open_usage_dashboard(&mut self) {
        let mut state = UsageDashboardState::new(UsagePeriod::Last7Days);
        self.spawn_usage_stats_request(&mut state);
        self.route_stack.clear();
        self.current_route = Route::Usage(state);
    }

    pub(crate) fn spawn_usage_stats_request(&mut self, state: &mut UsageDashboardState) {
        self.next_usage_request_id = self.next_usage_request_id.saturating_add(1);
        state.request_id = self.next_usage_request_id;
        state.loading = true;
        state.error = None;
        let request_id = state.request_id;
        let timezone_offset_minutes = chrono::Local::now().offset().local_minus_utc() / 60;
        let query = UsageStatsQuery::for_period_with_offset(
            state.period,
            Utc::now(),
            timezone_offset_minutes,
        )
        .with_filters(
            state
                .presentation
                .provider_filter
                .clone()
                .into_iter()
                .collect(),
            state
                .presentation
                .model_filter
                .clone()
                .into_iter()
                .collect(),
            Vec::new(),
            state.presentation.include_subagents,
        );
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .usage_stats(query)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::UsageStatsLoaded { request_id, result });
        });
    }

    pub(crate) fn handle_usage_stats_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<UsageStats>,
    ) {
        let Route::Usage(state) = &mut self.current_route else {
            return;
        };
        if request_id != state.request_id {
            return;
        }
        state.loading = false;
        match result {
            Ok(stats) => {
                if state.presentation.provider_filter.is_none()
                    && state.presentation.model_filter.is_none()
                {
                    state.available_providers = stats
                        .by_provider
                        .iter()
                        .map(|item| item.provider_id.clone())
                        .collect();
                    state.available_models = stats
                        .by_model
                        .iter()
                        .map(|item| (item.provider_id.clone(), item.model_id.clone()))
                        .collect();
                }
                state.data = Some(usage_dashboard_data(&stats));
                let row_count = state
                    .data
                    .as_ref()
                    .map(|data| data.row_count(state.presentation.view))
                    .unwrap_or_default();
                state.presentation.clamp_selection(row_count);
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    pub(crate) fn handle_usage_dashboard_key(
        &mut self,
        key: KeyEvent,
        state: &mut UsageDashboardState,
    ) -> bool {
        match resolve_tui_key(KeyContext::Usage, key) {
            Some(KeyAction::Close) => return true,
            Some(KeyAction::UsageCyclePeriod) => {
                self.activate_usage_control(state, UsageDashboardControl::Period);
            }
            Some(KeyAction::UsageCycleView) => {
                self.activate_usage_control(state, UsageDashboardControl::View);
            }
            Some(KeyAction::UsageCycleProvider) => {
                self.activate_usage_control(state, UsageDashboardControl::Provider);
            }
            Some(KeyAction::UsageCycleModel) => {
                self.activate_usage_control(state, UsageDashboardControl::Model);
            }
            Some(KeyAction::UsageToggleSubagents) => {
                self.activate_usage_control(state, UsageDashboardControl::Subagents);
            }
            Some(KeyAction::UsageCycleSort) => {
                self.activate_usage_control(state, UsageDashboardControl::Sort);
            }
            Some(KeyAction::Refresh) => {
                self.activate_usage_control(state, UsageDashboardControl::Refresh);
            }
            Some(KeyAction::MoveUp) => {
                let row_count = state
                    .data
                    .as_ref()
                    .map(|data| data.row_count(state.presentation.view))
                    .unwrap_or_default();
                state.presentation.move_selection(row_count, -1);
            }
            Some(KeyAction::MoveDown) => {
                let row_count = state
                    .data
                    .as_ref()
                    .map(|data| data.row_count(state.presentation.view))
                    .unwrap_or_default();
                state.presentation.move_selection(row_count, 1);
            }
            Some(KeyAction::Open) if state.presentation.view == UsageDashboardView::Sessions => {
                if let Some(session) = state
                    .data
                    .as_ref()
                    .and_then(|data| data.selected_session(&state.presentation))
                {
                    let session_id = session.session_id;
                    let title = session.title.clone();
                    self.open_session(session_id, title);
                    self.focus = Focus::Transcript;
                    return true;
                }
            }
            Some(_) | None => {}
        }
        false
    }

    fn activate_usage_control(
        &mut self,
        state: &mut UsageDashboardState,
        control: UsageDashboardControl,
    ) {
        let effect = state.presentation.activate(
            control,
            state.available_providers.as_slice(),
            state.available_models.as_slice(),
        );
        if effect == UsageDashboardEffect::CyclePeriod {
            state.period = cycle_usage_period(state.period, 1);
        }
        if matches!(
            effect,
            UsageDashboardEffect::CyclePeriod | UsageDashboardEffect::Reload
        ) {
            self.spawn_usage_stats_request(state);
        }
    }
}

pub(crate) fn usage_period_short_label(period: UsagePeriod) -> &'static str {
    match period {
        UsagePeriod::Today => "Today",
        UsagePeriod::Yesterday => "Yesterday",
        UsagePeriod::Last7Days => "7D",
        UsagePeriod::Last14Days => "14D",
        UsagePeriod::Last30Days => "30D",
        UsagePeriod::Last90Days => "90D",
        UsagePeriod::MonthToDate => "MTD",
        UsagePeriod::YearToDate => "YTD",
        UsagePeriod::AllTime => "All",
    }
}

fn usage_dashboard_data(stats: &UsageStats) -> UsageDashboardData {
    UsageDashboardData {
        totals: usage_dashboard_totals(&stats.totals),
        active_days: stats.active_days,
        average_cost_per_run_usd: stats.average_cost_per_run_usd,
        average_cost_per_active_day_usd: stats.average_cost_per_active_day_usd,
        peak_cost_usd: stats.peak_cost_usd,
        peak_cost_date: stats.peak_cost_date.clone(),
        daily: stats
            .by_day
            .iter()
            .map(|row| UsageDashboardRow {
                label: row.date.clone(),
                totals: usage_dashboard_totals(&row.totals),
                session: None,
            })
            .collect(),
        providers: stats
            .by_provider
            .iter()
            .map(|row| UsageDashboardRow {
                label: row.provider_id.clone(),
                totals: usage_dashboard_totals(&row.totals),
                session: None,
            })
            .collect(),
        models: stats
            .by_model
            .iter()
            .map(|row| UsageDashboardRow {
                label: format!("{} · {}", row.provider_id, row.model_id),
                totals: usage_dashboard_totals(&row.totals),
                session: None,
            })
            .collect(),
        sessions: stats
            .by_session
            .iter()
            .map(|row| UsageDashboardRow {
                label: format!(
                    "#{} {}{}",
                    row.session_id,
                    if row.is_subagent { "↳ " } else { "" },
                    row.title
                ),
                totals: usage_dashboard_totals(&row.totals),
                session: Some(UsageDashboardSessionLink {
                    session_id: row.session_id,
                    title: row.title.clone(),
                }),
            })
            .collect(),
    }
}

fn usage_dashboard_totals(totals: &UsageTotals) -> UsageDashboardTotals {
    UsageDashboardTotals {
        runs: totals.runs,
        sessions: totals.sessions,
        input_tokens: totals.input_tokens,
        output_tokens: totals.output_tokens,
        reasoning_tokens: totals.reasoning_tokens,
        cache_write_tokens: totals.cache_write_tokens,
        cache_write_5m_tokens: totals.cache_write_5m_tokens,
        cache_write_1h_tokens: totals.cache_write_1h_tokens,
        cache_read_tokens: totals.cache_read_tokens,
        tool_use_tokens: totals.tool_use_tokens,
        other_tokens: totals.other_tokens,
        total_tokens: totals.total_tokens,
        cache_hit_rate: totals.cache_hit_rate,
        total_cost_usd: totals.total_cost_usd,
        recorded_cost_usd: totals.recorded_cost_usd,
        estimated_cost_usd: totals.estimated_cost_usd,
        unpriced_runs: totals.unpriced_runs,
        billable_unit_kinds: totals.billable_units.len() as u64,
        unpriced_billable_units: totals
            .billable_units
            .iter()
            .map(|item| item.unpriced_quantity)
            .sum(),
    }
}

fn cycle_usage_period(current: UsagePeriod, delta: isize) -> UsagePeriod {
    let index = USAGE_PERIODS
        .iter()
        .position(|period| *period == current)
        .unwrap_or(2);
    let next = (index as isize + delta).rem_euclid(USAGE_PERIODS.len() as isize) as usize;
    USAGE_PERIODS[next]
}
