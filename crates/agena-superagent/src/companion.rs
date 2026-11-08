//! Workspace-scoped assistant preferences and explicitly authorized proactive
//! contact. Preferences are presentation data, never tool authority.
use crate::{Error, Result, validate_key};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssistantTone {
    #[default]
    Balanced,
    Warm,
    Direct,
    Playful,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistantPreferences {
    pub name: String,
    pub address_user: String,
    pub tone: AssistantTone,
    /// "auto" follows the current user's language; otherwise a language label.
    pub language: String,
    pub instructions: String,
    pub memory_enabled: bool,
    /// Allows only explicitly created reminders/check-ins, not invented jobs.
    pub proactive_enabled: bool,
    pub timezone: String,
    pub quiet_enabled: bool,
    pub quiet_start_minute: u16,
    pub quiet_end_minute: u16,
    pub daily_limit: u16,
    pub cooldown_seconds: u32,
}
impl Default for AssistantPreferences {
    fn default() -> Self {
        Self {
            name: "Agena".into(),
            address_user: String::new(),
            tone: AssistantTone::Balanced,
            language: "auto".into(),
            instructions: String::new(),
            memory_enabled: true,
            proactive_enabled: true,
            timezone: "UTC".into(),
            quiet_enabled: false,
            quiet_start_minute: 1320,
            quiet_end_minute: 480,
            daily_limit: 4,
            cooldown_seconds: 300,
        }
    }
}
impl AssistantPreferences {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.address_user.len() > 128
            || self.language.trim().is_empty()
            || self.language.len() > 64
            || self.instructions.len() > 4096
        {
            return Err(Error::Invalid(
                "assistant name/address/language/instructions exceed their bounds",
            ));
        }
        if self
            .name
            .chars()
            .chain(self.address_user.chars())
            .any(char::is_control)
        {
            return Err(Error::Invalid(
                "assistant names cannot contain control characters",
            ));
        }
        self.timezone.parse::<Tz>().map_err(|_| {
            Error::Invalid(
                "use a valid IANA timezone, for example Asia/Shanghai or America/Chicago",
            )
        })?;
        if self.quiet_start_minute >= 1440
            || self.quiet_end_minute >= 1440
            || (self.quiet_enabled && self.quiet_start_minute == self.quiet_end_minute)
            || !(1..=24).contains(&self.daily_limit)
            || !(60..=86400).contains(&self.cooldown_seconds)
        {
            return Err(Error::Invalid(
                "invalid quiet hours, daily limit (1–24), or cooldown (60–86400 seconds)",
            ));
        }
        Ok(())
    }
    pub fn quiet_at(&self, utc: DateTime<Utc>) -> bool {
        if !self.quiet_enabled {
            return false;
        }
        let Ok(tz) = self.timezone.parse::<Tz>() else {
            return true;
        };
        let local = utc.with_timezone(&tz);
        let minute = (local.hour() * 60 + local.minute()) as u16;
        if self.quiet_start_minute < self.quiet_end_minute {
            minute >= self.quiet_start_minute && minute < self.quiet_end_minute
        } else {
            minute >= self.quiet_start_minute || minute < self.quiet_end_minute
        }
    }
    pub fn local_day(&self, now_ms: i64) -> Result<String> {
        let tz = self
            .timezone
            .parse::<Tz>()
            .map_err(|_| Error::Invalid("invalid timezone"))?;
        let date = Utc
            .timestamp_millis_opt(now_ms)
            .single()
            .ok_or(Error::Invalid("invalid time"))?
            .with_timezone(&tz);
        Ok(format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month(),
            date.day()
        ))
    }
    /// UTC iteration handles nonexistent/repeated DST local minutes without
    /// inventing an ambiguous local timestamp. At most 26h of bounded work.
    pub fn next_allowed_ms(&self, now_ms: i64) -> Result<i64> {
        let now = Utc
            .timestamp_millis_opt(now_ms)
            .single()
            .ok_or(Error::Invalid("invalid time"))?;
        if !self.quiet_at(now) {
            return Ok(now_ms);
        }
        let start = now
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .ok_or(Error::Invalid("invalid time"))?;
        for minute in 1..=1560 {
            let next = start + Duration::minutes(minute);
            if !self.quiet_at(next) {
                return Ok(next.timestamp_millis());
            }
        }
        Err(Error::Invalid(
            "quiet hours did not produce a bounded next opening",
        ))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AssistantPatch {
    pub name: Option<String>,
    pub address_user: Option<String>,
    pub tone: Option<AssistantTone>,
    pub language: Option<String>,
    pub instructions: Option<String>,
    pub memory_enabled: Option<bool>,
    pub proactive_enabled: Option<bool>,
    pub timezone: Option<String>,
    pub quiet_enabled: Option<bool>,
    pub quiet_start_minute: Option<u16>,
    pub quiet_end_minute: Option<u16>,
    pub daily_limit: Option<u16>,
    pub cooldown_seconds: Option<u32>,
}
impl AssistantPatch {
    pub fn apply(&self, p: &mut AssistantPreferences) -> Result<()> {
        macro_rules! fields {($($field:ident),*)=>{$(if let Some(v)=&self.$field {p.$field=v.clone();})*};}
        fields!(
            name,
            address_user,
            tone,
            language,
            instructions,
            memory_enabled,
            proactive_enabled,
            timezone,
            quiet_enabled,
            quiet_start_minute,
            quiet_end_minute,
            daily_limit,
            cooldown_seconds
        );
        p.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AssistantProfile {
    pub workspace_id: i64,
    pub revision: i64,
    pub preferences: AssistantPreferences,
    pub updated_at_ms: i64,
}
impl AssistantProfile {
    pub fn empty(workspace_id: i64) -> Self {
        Self {
            workspace_id,
            revision: 0,
            preferences: Default::default(),
            updated_at_ms: 0,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistantUpdate {
    pub expected_revision: i64,
    pub request_key: String,
    pub patch: AssistantPatch,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    #[default]
    Reminder,
    CheckIn,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReminderState {
    Active,
    Paused,
    Delivering,
    Completed,
    Cancelled,
}
impl ReminderState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Delivering => "delivering",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReminderSpec {
    pub text: String,
    #[serde(default)]
    pub kind: ReminderKind,
    /// Relative reminders are calculated by the host, never by a guessed clock.
    #[serde(default)]
    pub after_seconds: Option<u32>,
    /// RFC3339 with explicit UTC offset. Use profile timezone to resolve natural dates.
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub repeat_seconds: Option<u32>,
    /// Mandatory finite cap for recurring contact; default one occurrence.
    #[serde(default = "one")]
    pub occurrences: u16,
}
fn one() -> u16 {
    1
}
impl ReminderSpec {
    pub fn due_ms(&self, now: i64) -> Result<i64> {
        if self.text.trim().is_empty()
            || self.text.len() > 4096
            || !(1..=365).contains(&self.occurrences)
        {
            return Err(Error::Invalid(
                "reminder text must contain 1–4096 bytes, with 1–365 occurrences",
            ));
        }
        if self
            .repeat_seconds
            .is_some_and(|s| !(3600..=31536000).contains(&s))
            || (self.occurrences > 1 && self.repeat_seconds.is_none())
        {
            return Err(Error::Invalid(
                "recurring contact requires an interval of at least one hour and a finite occurrence count",
            ));
        }
        let due = match (self.after_seconds, self.at.as_deref()) {
            (Some(seconds), None) if (1..=31536000).contains(&seconds) => now
                .checked_add(i64::from(seconds) * 1000)
                .ok_or(Error::Invalid("reminder time overflow"))?,
            (None, Some(at)) => DateTime::parse_from_rfc3339(at)
                .map_err(|_| {
                    Error::Invalid("scheduled time needs RFC3339 with an explicit offset")
                })?
                .timestamp_millis(),
            _ => return Err(Error::Invalid("specify exactly one of after_seconds or at")),
        };
        if due <= now || due > now + 366 * 86400000 {
            return Err(Error::Invalid(
                "reminder must be in the future and within 366 days",
            ));
        }
        Ok(due)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Reminder {
    #[schemars(with = "String")]
    pub id: Uuid,
    pub workspace_id: i64,
    pub source_session_id: i64,
    pub origin_message_id: i64,
    pub request_key: String,
    pub spec: ReminderSpec,
    pub state: ReminderState,
    pub revision: i64,
    pub next_due_ms: i64,
    pub occurrence: u16,
    pub delivered: u16,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub retry_at_ms: i64,
    pub delivery_text: Option<String>,
    #[serde(default)]
    pub reserved_day: Option<String>,
    #[serde(default)]
    pub reserved_at_ms: i64,
    pub last_error: Option<String>,
}
impl Reminder {
    pub fn key(&self) -> String {
        format!("companion:{}:{}", self.id, self.occurrence)
    }
    pub fn new(
        workspace: i64,
        source: i64,
        origin: i64,
        key: String,
        spec: ReminderSpec,
        now: i64,
    ) -> Result<Self> {
        validate_key(&key)?;
        let due = spec.due_ms(now)?;
        Ok(Self {
            id: Uuid::new_v4(),
            workspace_id: workspace,
            source_session_id: source,
            origin_message_id: origin,
            request_key: key,
            spec,
            state: ReminderState::Active,
            revision: 1,
            next_due_ms: due,
            occurrence: 1,
            delivered: 0,
            created_at_ms: now,
            updated_at_ms: now,
            retry_at_ms: 0,
            delivery_text: None,
            reserved_day: None,
            reserved_at_ms: 0,
            last_error: None,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReminderControl {
    pub expected_revision: i64,
    pub action: ReminderControlAction,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReminderControlAction {
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AssistantMemory {
    pub name: String,
    pub description: String,
    pub memory_type: Option<String>,
    pub body: String,
    pub sha256: String,
    pub truncated: bool,
    #[serde(default)]
    pub metadata: agena_domain::MemoryMetadata,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssistantMemoryUpdate {
    pub name: String,
    pub body: Option<String>,
    pub description: Option<String>,
    pub memory_type: Option<String>,
    pub expected_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AssistantSnapshot {
    pub source_session_id: i64,
    pub profile: AssistantProfile,
    pub reminders: Vec<Reminder>,
    pub memories: Vec<AssistantMemory>,
    pub memories_truncated: bool,
    pub now_ms: i64,
}

#[async_trait]
pub trait CompanionRepository: Send + Sync {
    async fn profile(&self, workspace: i64) -> Result<AssistantProfile>;
    async fn update_profile(
        &self,
        workspace: i64,
        update: AssistantUpdate,
        now: i64,
    ) -> Result<AssistantProfile>;
    async fn create_reminder(&self, reminder: Reminder) -> Result<Reminder>;
    async fn reminder(&self, id: Uuid) -> Result<Reminder>;
    async fn reminders(&self, workspace: i64) -> Result<Vec<Reminder>>;
    async fn due_reminders(&self, now: i64) -> Result<Vec<Reminder>>;
    /// Atomically reserve a local-day budget slot and freeze a notice before I/O.
    async fn reserve_reminder(&self, id: Uuid, now: i64) -> Result<Option<Reminder>>;
    async fn finish_reminder(&self, id: Uuid, occurrence: u16, now: i64) -> Result<()>;
    async fn defer_reminder(&self, id: Uuid, now: i64, error: &str) -> Result<()>;
    async fn control_reminder(
        &self,
        id: Uuid,
        control: ReminderControl,
        now: i64,
    ) -> Result<Reminder>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timezone_quiet_hours_cross_midnight_and_dst() {
        let mut p = AssistantPreferences {
            timezone: "America/Chicago".into(),
            quiet_enabled: true,
            quiet_start_minute: 1320,
            quiet_end_minute: 480,
            ..Default::default()
        };
        let now = DateTime::parse_from_rfc3339("2026-11-01T00:30:00-05:00")
            .unwrap()
            .timestamp_millis();
        let next = p.next_allowed_ms(now).unwrap();
        assert_eq!(
            Utc.timestamp_millis_opt(next)
                .unwrap()
                .with_timezone(&"America/Chicago".parse::<Tz>().unwrap())
                .hour(),
            8
        );
        assert!(next - now > 7 * 3600000);
        p.quiet_start_minute = 600;
        p.quiet_end_minute = 660;
        assert!(p.validate().is_ok());
    }
    #[test]
    fn bounded_schedule_and_policy_validation() {
        let p = AssistantPreferences {
            timezone: "not-a-zone".into(),
            ..Default::default()
        };
        assert!(p.validate().is_err());
        let mut spec = ReminderSpec {
            text: "检查结果".into(),
            kind: ReminderKind::CheckIn,
            after_seconds: Some(1),
            at: None,
            repeat_seconds: None,
            occurrences: 1,
        };
        assert_eq!(spec.due_ms(1000).unwrap(), 2000);
        spec.occurrences = 2;
        assert!(spec.due_ms(1000).is_err());
        spec.repeat_seconds = Some(3600);
        assert!(spec.due_ms(1000).is_ok());
    }
}
