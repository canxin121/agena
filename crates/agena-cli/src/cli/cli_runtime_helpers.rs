use super::{
    AppError, ContinueArgs, DateTime, Duration, Instant, ModelRef, Role, SessionDetail,
    SessionRunOptions, UsageArgs, UsageStatsQuery, Utc,
};
use agena_provider::ProviderCatalog;

pub(super) fn usage_stats_query_from_args(args: &UsageArgs) -> Result<UsageStatsQuery, AppError> {
    let has_custom_range = args.from.is_some() || args.to.is_some();
    let mut query = UsageStatsQuery::for_period_with_offset(
        args.period.into_usage_period(),
        Utc::now(),
        args.timezone_offset_minutes,
    );
    if has_custom_range {
        query = UsageStatsQuery::custom(
            args.from
                .as_deref()
                .map(|value| parse_usage_datetime(value, false))
                .transpose()?,
            args.to
                .as_deref()
                .map(|value| parse_usage_datetime(value, true))
                .transpose()?
                .or_else(|| Some(Utc::now())),
        )
        .with_timezone_offset(args.timezone_offset_minutes);
    }

    if let (Some(from), Some(to)) = (query.from.as_ref(), query.to.as_ref())
        && from > to
    {
        return Err(AppError::Config(
            "--from must be earlier than or equal to --to".to_string(),
        ));
    }

    Ok(query.with_filters(
        args.provider.clone(),
        args.model.clone(),
        args.session.clone(),
        args.include_subagents,
    ))
}

pub(super) fn parse_usage_datetime(raw: &str, end_of_day: bool) -> Result<DateTime<Utc>, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config("usage date cannot be empty".to_string()));
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Utc));
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let datetime = if end_of_day {
            date.and_hms_milli_opt(23, 59, 59, 999)
        } else {
            date.and_hms_milli_opt(0, 0, 0, 0)
        }
        .expect("valid date boundary");
        return Ok(datetime.and_utc());
    }

    Err(AppError::Config(format!(
        "invalid usage date `{raw}`; expected YYYY-MM-DD or RFC3339"
    )))
}

pub(super) async fn latest_event_seq(
    queries: &dyn agena_runtime::SessionQueryService,
    session_id: i64,
) -> Result<Option<i64>, AppError> {
    queries
        .latest_event_seq(session_id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))
}

pub(super) fn session_detail_from_presentation(
    session: agena_runtime::SessionPresentation,
    latest_event_seq: Option<i64>,
) -> SessionDetail {
    SessionDetail {
        id: session.id,
        parent_id: session.parent_id,
        workspace_id: session.workspace_id,
        title: session.title,
        version: session.version,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: session.message_count,
        status: session.workflow_state,
        latest_event_seq,
    }
}

pub(super) async fn resolve_continue_options(
    providers: &dyn ProviderCatalog,
    control: &dyn agena_runtime::SessionExecutionControl,
    session_id: i64,
    args: &ContinueArgs,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = args.model.as_deref() {
        providers
            .resolve_model_target(model, None)
            .map_err(|error| AppError::Config(error.to_string()))?
    } else if let Some(model) = control
        .selected_model(session_id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
    {
        model
    } else {
        default_model(providers)?
    };

    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: args.temperature,
        max_output_tokens: args.max_output_tokens,
    })
}

pub(super) fn resolve_run_options(
    providers: &dyn ProviderCatalog,
    model: Option<&str>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = model {
        providers
            .resolve_model_target(model, None)
            .map_err(|error| AppError::Config(error.to_string()))?
    } else {
        default_model(providers)?
    };

    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature,
        max_output_tokens,
    })
}

pub(super) fn default_model(providers: &dyn ProviderCatalog) -> Result<ModelRef, AppError> {
    providers
        .default_model()
        .map_err(|error| AppError::Config(error.to_string()))?
        .ok_or_else(|| AppError::Config("no providers configured".to_owned()))
}

pub(super) fn last_assistant_text_from_projection(
    runs: Vec<agena_runtime::SessionProjectedRun>,
) -> Option<String> {
    runs.into_iter()
        .rev()
        .find(|run| run.role == Role::Assistant)
        .map(|run| projected_run_visible_text(&run))
        .filter(|text| !text.trim().is_empty())
}

pub(super) fn projected_run_visible_text(run: &agena_runtime::SessionProjectedRun) -> String {
    run.parts
        .iter()
        .filter_map(|part| match part.detail.as_ref() {
            Some(agena_runtime::SessionProjectedPartDetail::Text { text, .. }) => {
                Some(text.clone())
            }
            _ => part.summary.clone(),
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn title_from_prompt(prompt: &str) -> String {
    let title = prompt.trim().replace('\n', " ");
    let mut chars = title.chars();
    let truncated = chars.by_ref().take(80).collect::<String>();
    if truncated.is_empty() {
        "exec".to_owned()
    } else if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

pub(super) fn session_storage_error() -> AppError {
    AppError::Config("session storage is unavailable; configure a database URL or path".to_owned())
}

pub(super) async fn poll_until<T, F, Fut>(
    timeout: Duration,
    interval: Duration,
    mut poll: F,
) -> Result<Option<T>, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, AppError>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll().await? {
            return Ok(Some(value));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(interval).await;
    }
}
