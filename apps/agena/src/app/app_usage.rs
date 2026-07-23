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
    pub(in crate::app) fn open_usage_dashboard(&mut self, args: &str) {
        let (period, provider_filter, model_filter, include_subagents) =
            match parse_usage_dashboard_args(args) {
                Ok(parsed) => parsed,
                Err(usage) => {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &agena_tui::fl_args!("usage" => usage),
                    ));
                    return;
                }
            };
        let mut state = UsageDashboardState::new(period);
        state.presentation.provider_filter = provider_filter;
        state.presentation.model_filter = model_filter;
        state.presentation.include_subagents = include_subagents;
        self.spawn_usage_stats_request(&mut state);
        self.route_stack.clear();
        self.current_route = Route::Usage(state);
    }

    pub(in crate::app) fn spawn_usage_stats_request(&mut self, state: &mut UsageDashboardState) {
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
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::UsageStatsLoaded { request_id, result });
        });
    }

    pub(in crate::app) fn handle_usage_stats_loaded(
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
            Err(error) => state.error = Some(error),
        }
    }

    pub(in crate::app) fn handle_usage_dashboard_key(
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

pub(in crate::app) fn usage_period_short_label(period: UsagePeriod) -> &'static str {
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
        cache_write_tokens: totals.cache_write_tokens,
        cache_read_tokens: totals.cache_read_tokens,
        total_tokens: totals.total_tokens,
        cache_hit_rate: totals.cache_hit_rate,
        total_cost_usd: totals.total_cost_usd,
        recorded_cost_usd: totals.recorded_cost_usd,
        estimated_cost_usd: totals.estimated_cost_usd,
        unpriced_runs: totals.unpriced_runs,
    }
}

fn parse_usage_dashboard_args(
    args: &str,
) -> Result<(UsagePeriod, Option<String>, Option<String>, bool), &'static str> {
    let mut period = UsagePeriod::Last7Days;
    let mut provider = None;
    let mut model = None;
    let mut include_subagents = true;
    let mut tokens = args.split_whitespace();
    while let Some(token) = tokens.next() {
        match token.to_ascii_lowercase().as_str() {
            "today" | "1d" => period = UsagePeriod::Today,
            "yesterday" | "yd" => period = UsagePeriod::Yesterday,
            "week" | "7d" => period = UsagePeriod::Last7Days,
            "2w" | "14d" => period = UsagePeriod::Last14Days,
            "30d" => period = UsagePeriod::Last30Days,
            "90d" => period = UsagePeriod::Last90Days,
            "month" | "mtd" => period = UsagePeriod::MonthToDate,
            "year" | "ytd" => period = UsagePeriod::YearToDate,
            "all" | "all-time" => period = UsagePeriod::AllTime,
            "--provider" | "-p" => {
                provider = Some(
                    tokens
                        .next()
                        .ok_or(usage_dashboard_invocation())?
                        .to_string(),
                )
            }
            "--model" | "-m" => {
                model = Some(
                    tokens
                        .next()
                        .ok_or(usage_dashboard_invocation())?
                        .to_string(),
                )
            }
            "--no-subagents" => include_subagents = false,
            _ => return Err(usage_dashboard_invocation()),
        }
    }
    Ok((period, provider, model, include_subagents))
}

fn usage_dashboard_invocation() -> &'static str {
    "/usage [today|yesterday|7d|14d|30d|90d|month|year|all] [--provider ID] [--model ID] [--no-subagents]"
}

fn cycle_usage_period(current: UsagePeriod, delta: isize) -> UsagePeriod {
    let index = USAGE_PERIODS
        .iter()
        .position(|period| *period == current)
        .unwrap_or(2);
    let next = (index as isize + delta).rem_euclid(USAGE_PERIODS.len() as isize) as usize;
    USAGE_PERIODS[next]
}

#[cfg(test)]
mod tests {
    use super::{UsagePeriod, parse_usage_dashboard_args};

    #[test]
    fn parses_usage_period_and_filters() {
        let parsed =
            parse_usage_dashboard_args("30d --provider openai --model gpt-5 --no-subagents")
                .expect("valid usage args");
        assert_eq!(parsed.0, UsagePeriod::Last30Days);
        assert_eq!(parsed.1.as_deref(), Some("openai"));
        assert_eq!(parsed.2.as_deref(), Some("gpt-5"));
        assert!(!parsed.3);
    }

    #[test]
    fn rejects_unknown_usage_arguments() {
        assert!(parse_usage_dashboard_args("banana").is_err());
    }
}
