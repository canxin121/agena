use agena::session::{SessionUsageBreakdown, UsagePeriod, UsageStatsQuery};

use super::{
    App, AppMessage, Focus, KeyEvent, Route, UsageDashboardControl, UsageDashboardSort,
    UsageDashboardState, UsageDashboardView, Utc,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};

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
                    self.flash_warning(
                        self.i18n
                            .text_args("flash-command-usage", &crate::fl_args!("usage" => usage)),
                    );
                    return;
                }
            };
        let mut state = UsageDashboardState::new(period);
        state.provider_filter = provider_filter;
        state.model_filter = model_filter;
        state.include_subagents = include_subagents;
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
            state.provider_filter.clone().into_iter().collect(),
            state.model_filter.clone().into_iter().collect(),
            Vec::new(),
            state.include_subagents,
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
        result: super::UiResult<agena::session::UsageStats>,
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
                if state.provider_filter.is_none() && state.model_filter.is_none() {
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
                state.stats = Some(stats);
                state.selected = state
                    .selected
                    .min(usage_dashboard_row_count(state).saturating_sub(1));
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
            Some(KeyAction::NextTab) => {
                move_usage_focus(state, 1);
            }
            Some(KeyAction::PreviousTab) => {
                move_usage_focus(state, -1);
            }
            Some(KeyAction::MoveUp) if !state.controls_focused => move_usage_selection(state, -1),
            Some(KeyAction::MoveDown) if !state.controls_focused => move_usage_selection(state, 1),
            Some(KeyAction::Home) if !state.controls_focused => {
                state.selected = 0;
                state.scroll = 0;
            }
            Some(KeyAction::End) if !state.controls_focused => {
                state.selected = usage_dashboard_row_count(state).saturating_sub(1);
            }
            Some(KeyAction::Open) if state.controls_focused => {
                self.activate_usage_control(state);
            }
            Some(KeyAction::Open) if state.view == UsageDashboardView::Sessions => {
                if let Some(session) = selected_usage_session(state) {
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

    fn activate_usage_control(&mut self, state: &mut UsageDashboardState) {
        let control = UsageDashboardControl::ALL
            .get(state.selected_control)
            .copied()
            .unwrap_or(UsageDashboardControl::Period);
        let mut reload = false;
        match control {
            UsageDashboardControl::Period => {
                state.period = cycle_usage_period(state.period, 1);
                reload = true;
            }
            UsageDashboardControl::View => {
                state.view = state.view.cycle(1);
            }
            UsageDashboardControl::Provider => {
                cycle_provider_filter(state, 1);
                reload = true;
            }
            UsageDashboardControl::Model => {
                cycle_model_filter(state, 1);
                reload = true;
            }
            UsageDashboardControl::Subagents => {
                state.include_subagents = !state.include_subagents;
                reload = true;
            }
            UsageDashboardControl::Sort => {
                state.sort = state.sort.next();
            }
            UsageDashboardControl::Refresh => reload = true,
        }
        reset_usage_selection(state);
        if reload {
            self.spawn_usage_stats_request(state);
        }
    }
}

fn move_usage_focus(state: &mut UsageDashboardState, delta: isize) {
    let control_count = UsageDashboardControl::ALL.len();
    if delta > 0 {
        if !state.controls_focused {
            state.controls_focused = true;
            state.selected_control = 0;
        } else if state.selected_control + 1 < control_count {
            state.selected_control += 1;
        } else {
            state.controls_focused = false;
        }
    } else if !state.controls_focused {
        state.controls_focused = true;
        state.selected_control = control_count.saturating_sub(1);
    } else if state.selected_control > 0 {
        state.selected_control -= 1;
    } else {
        state.controls_focused = false;
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

pub(in crate::app) fn usage_dashboard_view_label(view: UsageDashboardView) -> &'static str {
    match view {
        UsageDashboardView::Overview => "Overview",
        UsageDashboardView::Daily => "Daily",
        UsageDashboardView::Providers => "Providers",
        UsageDashboardView::Models => "Models",
        UsageDashboardView::Sessions => "Sessions",
    }
}

pub(in crate::app) fn usage_dashboard_sort_label(sort: UsageDashboardSort) -> &'static str {
    match sort {
        UsageDashboardSort::Cost => "Cost",
        UsageDashboardSort::Tokens => "Tokens",
        UsageDashboardSort::Runs => "Runs",
    }
}

pub(in crate::app) fn usage_sorted_sessions(
    state: &UsageDashboardState,
) -> Vec<&SessionUsageBreakdown> {
    let mut rows = state
        .stats
        .as_ref()
        .map(|stats| stats.by_session.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    rows.sort_by(|left, right| usage_sort_order(state.sort, &left.totals, &right.totals));
    rows
}

pub(in crate::app) fn usage_sort_order(
    sort: UsageDashboardSort,
    left: &agena::session::UsageTotals,
    right: &agena::session::UsageTotals,
) -> std::cmp::Ordering {
    match sort {
        UsageDashboardSort::Cost => right
            .total_cost_usd
            .partial_cmp(&left.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal),
        UsageDashboardSort::Tokens => right.total_tokens.cmp(&left.total_tokens),
        UsageDashboardSort::Runs => right.runs.cmp(&left.runs),
    }
    .then(right.runs.cmp(&left.runs))
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

fn reset_usage_selection(state: &mut UsageDashboardState) {
    state.selected = 0;
    state.scroll = 0;
}

fn move_usage_selection(state: &mut UsageDashboardState, delta: isize) {
    let len = usage_dashboard_row_count(state);
    if len == 0 {
        state.selected = 0;
        return;
    }
    state.selected = (state.selected as isize + delta).clamp(0, len as isize - 1) as usize;
}

fn usage_dashboard_row_count(state: &UsageDashboardState) -> usize {
    let Some(stats) = state.stats.as_ref() else {
        return 0;
    };
    match state.view {
        UsageDashboardView::Overview => 0,
        UsageDashboardView::Daily => stats.by_day.len(),
        UsageDashboardView::Providers => stats.by_provider.len(),
        UsageDashboardView::Models => stats.by_model.len(),
        UsageDashboardView::Sessions => stats.by_session.len(),
    }
}

fn cycle_provider_filter(state: &mut UsageDashboardState, delta: isize) {
    let mut choices = Vec::with_capacity(state.available_providers.len() + 1);
    choices.push(None);
    choices.extend(state.available_providers.iter().cloned().map(Some));
    let index = choices
        .iter()
        .position(|choice| choice == &state.provider_filter)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(choices.len() as isize) as usize;
    state.provider_filter = choices[next].clone();
    state.model_filter = None;
}

fn cycle_model_filter(state: &mut UsageDashboardState, delta: isize) {
    let models = state
        .available_models
        .iter()
        .filter(|(provider, _)| {
            state
                .provider_filter
                .as_ref()
                .is_none_or(|selected| selected == provider)
        })
        .map(|(_, model)| model.clone())
        .collect::<Vec<_>>();
    let mut choices = Vec::with_capacity(models.len() + 1);
    choices.push(None);
    for model in models {
        if !choices.contains(&Some(model.clone())) {
            choices.push(Some(model));
        }
    }
    let index = choices
        .iter()
        .position(|choice| choice == &state.model_filter)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(choices.len() as isize) as usize;
    state.model_filter = choices[next].clone();
}

fn selected_usage_session(state: &UsageDashboardState) -> Option<&SessionUsageBreakdown> {
    usage_sorted_sessions(state).get(state.selected).copied()
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
