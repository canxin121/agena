use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Utc};

use crate::UsagePeriod;

/// A transport- and storage-neutral request for session usage statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageStatsQuery {
    pub period: UsagePeriod,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub provider_ids: Vec<String>,
    pub model_ids: Vec<String>,
    pub session_ids: Vec<i64>,
    pub include_subagents: bool,
    pub timezone_offset_minutes: i32,
}

impl UsageStatsQuery {
    pub fn for_period(period: UsagePeriod, now: DateTime<Utc>) -> Self {
        Self::for_period_with_offset(period, now, 0)
    }

    pub fn for_period_with_offset(
        period: UsagePeriod,
        now: DateTime<Utc>,
        timezone_offset_minutes: i32,
    ) -> Self {
        let timezone_offset_minutes = clamp_timezone_offset(timezone_offset_minutes);
        let offset = fixed_offset(timezone_offset_minutes);
        let local_now = now.with_timezone(&offset);
        let today = start_of_local_day(local_now, offset);
        let (from, to) = match period {
            UsagePeriod::Today => (Some(today), Some(now)),
            UsagePeriod::Yesterday => (
                Some(today - Duration::days(1)),
                Some(today - Duration::milliseconds(1)),
            ),
            UsagePeriod::Last7Days => (Some(today - Duration::days(6)), Some(now)),
            UsagePeriod::Last14Days => (Some(today - Duration::days(13)), Some(now)),
            UsagePeriod::Last30Days => (Some(today - Duration::days(29)), Some(now)),
            UsagePeriod::Last90Days => (Some(today - Duration::days(89)), Some(now)),
            UsagePeriod::MonthToDate => (Some(start_of_local_month(local_now, offset)), Some(now)),
            UsagePeriod::YearToDate => (Some(start_of_local_year(local_now, offset)), Some(now)),
            UsagePeriod::AllTime => (None, Some(now)),
        };
        Self {
            period,
            from,
            to,
            provider_ids: Vec::new(),
            model_ids: Vec::new(),
            session_ids: Vec::new(),
            include_subagents: true,
            timezone_offset_minutes,
        }
    }

    pub fn custom(from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Self {
        Self {
            period: UsagePeriod::AllTime,
            from,
            to,
            provider_ids: Vec::new(),
            model_ids: Vec::new(),
            session_ids: Vec::new(),
            include_subagents: true,
            timezone_offset_minutes: 0,
        }
    }

    pub fn with_timezone_offset(mut self, timezone_offset_minutes: i32) -> Self {
        self.timezone_offset_minutes = clamp_timezone_offset(timezone_offset_minutes);
        self
    }

    pub fn with_filters(
        mut self,
        provider_ids: Vec<String>,
        model_ids: Vec<String>,
        mut session_ids: Vec<i64>,
        include_subagents: bool,
    ) -> Self {
        self.provider_ids = normalized_filters(provider_ids);
        self.model_ids = normalized_filters(model_ids);
        session_ids.sort_unstable();
        session_ids.dedup();
        self.session_ids = session_ids;
        self.include_subagents = include_subagents;
        self
    }

    pub fn matches(
        &self,
        session_id: i64,
        is_subagent: bool,
        provider_id: &str,
        model_id: &str,
    ) -> bool {
        (self.include_subagents || !is_subagent)
            && (self.session_ids.is_empty() || self.session_ids.contains(&session_id))
            && matches_text_filter(self.provider_ids.as_slice(), provider_id)
            && matches_text_filter(self.model_ids.as_slice(), model_id)
    }
}

fn start_of_local_day(now: DateTime<FixedOffset>, offset: FixedOffset) -> DateTime<Utc> {
    offset
        .from_local_datetime(
            &now.date_naive()
                .and_hms_opt(0, 0, 0)
                .expect("midnight is valid"),
        )
        .single()
        .expect("fixed offsets have unambiguous local datetimes")
        .with_timezone(&Utc)
}

fn start_of_local_month(now: DateTime<FixedOffset>, offset: FixedOffset) -> DateTime<Utc> {
    offset
        .from_local_datetime(
            &now.date_naive()
                .with_day(1)
                .expect("day 1 is valid")
                .and_hms_opt(0, 0, 0)
                .expect("midnight is valid"),
        )
        .single()
        .expect("fixed offsets have unambiguous local datetimes")
        .with_timezone(&Utc)
}

fn start_of_local_year(now: DateTime<FixedOffset>, offset: FixedOffset) -> DateTime<Utc> {
    offset
        .from_local_datetime(
            &now.date_naive()
                .with_month(1)
                .and_then(|date| date.with_day(1))
                .expect("January 1 is valid")
                .and_hms_opt(0, 0, 0)
                .expect("midnight is valid"),
        )
        .single()
        .expect("fixed offsets have unambiguous local datetimes")
        .with_timezone(&Utc)
}

fn fixed_offset(timezone_offset_minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(clamp_timezone_offset(timezone_offset_minutes) * 60)
        .expect("clamped timezone offset is valid")
}

fn clamp_timezone_offset(timezone_offset_minutes: i32) -> i32 {
    timezone_offset_minutes.clamp(-1_439, 1_439)
}

fn normalized_filters(filters: Vec<String>) -> Vec<String> {
    let mut filters = filters
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    filters.sort();
    filters.dedup();
    filters
}

fn matches_text_filter(filters: &[String], value: &str) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| value.eq_ignore_ascii_case(filter))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{UsagePeriod, UsageStatsQuery};

    #[test]
    fn timezone_aligned_period_and_filters_are_stable() {
        let now = Utc.with_ymd_and_hms(2026, 7, 11, 3, 30, 0).unwrap();
        let query = UsageStatsQuery::for_period_with_offset(UsagePeriod::Today, now, 480)
            .with_filters(
                vec![" OpenAI ".to_owned(), "openai".to_owned()],
                vec!["gpt-5".to_owned()],
                vec![3, 1, 3],
                false,
            );

        assert_eq!(
            query.from,
            Some(Utc.with_ymd_and_hms(2026, 7, 10, 16, 0, 0).unwrap())
        );
        assert_eq!(query.provider_ids, ["openai"]);
        assert_eq!(query.session_ids, [1, 3]);
        assert!(query.matches(1, false, "OPENAI", "gpt-5"));
        assert!(!query.matches(1, true, "openai", "gpt-5"));
    }
}
