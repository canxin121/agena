//! Stable usage-reporting period values.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
/// Period over which usage is aggregated.
pub enum UsagePeriod {
    Today,
    Yesterday,
    #[serde(rename = "last_7_days")]
    Last7Days,
    #[serde(rename = "last_14_days")]
    Last14Days,
    #[serde(rename = "last_30_days")]
    Last30Days,
    #[serde(rename = "last_90_days")]
    Last90Days,
    MonthToDate,
    YearToDate,
    AllTime,
}

impl UsagePeriod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Yesterday => "yesterday",
            Self::Last7Days => "last_7_days",
            Self::Last14Days => "last_14_days",
            Self::Last30Days => "last_30_days",
            Self::Last90Days => "last_90_days",
            Self::MonthToDate => "month_to_date",
            Self::YearToDate => "year_to_date",
            Self::AllTime => "all_time",
        }
    }
}
