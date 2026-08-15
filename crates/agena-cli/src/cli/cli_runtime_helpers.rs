use super::{AppError, DateTime, UsageArgs, UsageStatsQuery, Utc};

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
