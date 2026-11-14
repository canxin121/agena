//! Usage filter normalization and querying shared by every transport.
use super::wire;
use crate::{Application, ApplicationError};
use agena_api::management::UsageQuery;
use agena_domain::{UsagePeriod, UsageStatsQuery};

impl Application {
    pub async fn query_usage_stats(
        &self,
        query: UsageQuery,
    ) -> Result<serde_json::Value, ApplicationError> {
        let query = usage_stats_query(query)?;
        let services = self.session_execution_services()?;
        let usage = services
            .queries
            .usage_stats(query)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        wire(usage)
    }
}

fn usage_stats_query(query: UsageQuery) -> Result<UsageStatsQuery, ApplicationError> {
    let timezone_offset_minutes = query.timezone_offset_minutes.unwrap_or_default();
    if !(-1_439..=1_439).contains(&timezone_offset_minutes) {
        return Err(ApplicationError::bad_request(
            "The timezone offset must be between -1439 and 1439 minutes.",
        ));
    }
    let provider_ids = parse_usage_csv(query.provider.as_deref());
    let model_ids = parse_usage_csv(query.model.as_deref());
    let session_ids = parse_usage_session_ids(query.session.as_deref())?;
    let include_subagents = query.include_subagents.unwrap_or(true);
    let has_custom_range = query.from.is_some() || query.to.is_some();
    if has_custom_range {
        let from = query
            .from
            .as_deref()
            .map(|value| parse_usage_datetime(value, false))
            .transpose()?;
        let to = query
            .to
            .as_deref()
            .map(|value| parse_usage_datetime(value, true))
            .transpose()?
            .or_else(|| Some(chrono::Utc::now()));
        if let (Some(from), Some(to)) = (from.as_ref(), to.as_ref())
            && from > to
        {
            return Err(ApplicationError::bad_request(
                "The start date must be earlier than or equal to the end date.",
            ));
        }
        return Ok(UsageStatsQuery::custom(from, to)
            .with_timezone_offset(timezone_offset_minutes)
            .with_filters(provider_ids, model_ids, session_ids, include_subagents));
    }

    Ok(UsageStatsQuery::for_period_with_offset(
        query.period.unwrap_or(UsagePeriod::Last7Days),
        chrono::Utc::now(),
        timezone_offset_minutes,
    )
    .with_filters(provider_ids, model_ids, session_ids, include_subagents))
}

fn parse_usage_csv(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_usage_session_ids(raw: Option<&str>) -> Result<Vec<i64>, ApplicationError> {
    parse_usage_csv(raw)
        .into_iter()
        .map(|value| {
            value.parse::<i64>().map_err(|_| {
                ApplicationError::bad_request_with_diagnostic(
                    "A session ID in the usage filter is invalid.",
                    format!("invalid session id `{value}` in usage filter"),
                )
            })
        })
        .collect()
}

fn parse_usage_datetime(
    raw: &str,
    end_of_day: bool,
) -> Result<chrono::DateTime<chrono::Utc>, ApplicationError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApplicationError::bad_request(
            "The usage date cannot be empty.",
        ));
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&chrono::Utc));
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
    Err(ApplicationError::bad_request_with_diagnostic(
        "The usage date must use YYYY-MM-DD or RFC3339 format.",
        format!("invalid usage date `{raw}`"),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_custom_range_preserves_filters_and_date_boundaries() {
        let query = usage_stats_query(UsageQuery {
            from: Some("2026-01-02".into()),
            to: Some("2026-01-03".into()),
            provider: Some(" openai, ,anthropic ".into()),
            model: Some(" gpt-5,claude ".into()),
            session: Some(" 7, 3, 7 ".into()),
            include_subagents: Some(false),
            timezone_offset_minutes: Some(480),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            query.from.unwrap().to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );
        assert_eq!(
            query.to.unwrap().to_rfc3339(),
            "2026-01-03T23:59:59.999+00:00"
        );
        assert_eq!(query.provider_ids, ["anthropic", "openai"]);
        assert_eq!(query.model_ids, ["claude", "gpt-5"]);
        assert_eq!(query.session_ids, [3, 7]);
        assert!(!query.include_subagents);
        assert_eq!(query.timezone_offset_minutes, 480);
    }

    #[test]
    fn usage_rejects_invalid_ranges_and_session_identifiers_before_querying() {
        for query in [
            UsageQuery {
                timezone_offset_minutes: Some(1440),
                ..Default::default()
            },
            UsageQuery {
                session: Some("7,not-an-id".into()),
                ..Default::default()
            },
            UsageQuery {
                from: Some("2026-01-04".into()),
                to: Some("2026-01-03".into()),
                ..Default::default()
            },
            UsageQuery {
                from: Some(String::new()),
                ..Default::default()
            },
        ] {
            assert!(usage_stats_query(query).is_err());
        }
        let query = usage_stats_query(UsageQuery {
            from: Some("2026-01-02T08:00:00+08:00".into()),
            to: Some("2026-01-02T09:00:00+08:00".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            query.from.unwrap().to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );
        assert!(query.include_subagents);
    }
}
