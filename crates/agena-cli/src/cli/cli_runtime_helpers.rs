use std::fmt::Write;

use agena_domain::{UsageStats, format_usage_count};

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
        && from >= to
    {
        return Err(AppError::Config(
            "--from must be earlier than the end of the usage range".to_string(),
        ));
    }

    Ok(query.with_filters(
        args.provider.clone(),
        args.model.clone(),
        args.session.clone(),
        args.include_subagents,
    ))
}

fn parse_usage_datetime(raw: &str, upper: bool) -> Result<DateTime<Utc>, AppError> {
    if raw.trim().is_empty() {
        return Err(AppError::Config("usage date cannot be empty".to_string()));
    }

    agena_domain::parse_usage_date_boundary(raw, upper).ok_or_else(|| {
        AppError::Config(format!(
            "invalid usage date `{raw}`; expected YYYY-MM-DD or RFC3339"
        ))
    })
}

pub(super) fn render_usage_text(stats: &UsageStats, custom_range: bool) -> String {
    let period = if custom_range {
        "custom range".to_owned()
    } else {
        stats.period.label().replace('_', " ")
    };
    let totals = &stats.totals;
    let mut output = format!(
        "Usage ({period})\n{} tokens · {} requests · {} sessions\n\
         Uncached input {} · Output {} · Reasoning {}\n\
         Cache read {} · Cache write {} · Other {}\n\
         Cost ${:.6} (${:.6} recorded, ${:.6} estimated)",
        format_usage_count(totals.total_tokens),
        format_usage_count(totals.requests),
        format_usage_count(totals.sessions),
        format_usage_count(totals.input_tokens),
        format_usage_count(totals.output_tokens),
        format_usage_count(totals.reasoning_tokens),
        format_usage_count(totals.cache_read_tokens),
        format_usage_count(totals.cache_write_tokens),
        format_usage_count(totals.other_tokens),
        totals.total_cost_usd,
        totals.recorded_cost_usd,
        totals.estimated_cost_usd,
    );
    if totals.unpriced_requests > 0 {
        let _ = write!(
            output,
            "\n{} requests with incomplete pricing",
            format_usage_count(totals.unpriced_requests)
        );
    }
    for model in &stats.by_model {
        let _ = write!(
            output,
            "\n{}/{}  {} tokens · {} requests · ${:.6}",
            model.provider_id,
            model.model_id,
            format_usage_count(model.totals.total_tokens),
            format_usage_count(model.totals.requests),
            model.totals.total_cost_usd,
        );
    }
    output
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

pub(super) fn normalized_mcp_server_name(server: &str) -> Result<String, AppError> {
    let server = server.trim();
    if server.is_empty() {
        return Err(AppError::Config(
            "MCP server name must not be empty".to_owned(),
        ));
    }
    Ok(server.to_owned())
}
