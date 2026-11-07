//! Clock used by scheduler deadlines and job state. Ordinary processes use
//! system UTC. An opted-in debug Agena child reads a case-owned frozen clock
//! and publishes scheduler admission acknowledgements for deterministic tests.
//!
//! Tokio heartbeat, HTTP, and shutdown safety deadlines remain real. Only
//! scheduler wall time and the testkit's explicit virtual timers are controlled.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[cfg(debug_assertions)]
use std::sync::OnceLock;
use std::{path::PathBuf, time::Duration};

pub const TEST_CLOCK_ENV: &str = "AGENA_TEST_SCHEDULER_CLOCK_FILE";

#[cfg(all(test, debug_assertions))]
mod worker_tests;

fn controlled_path() -> Option<&'static PathBuf> {
    #[cfg(debug_assertions)]
    {
        static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
        PATH.get_or_init(|| std::env::var_os(TEST_CLOCK_ENV).map(PathBuf::from))
            .as_ref()
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Wire format shared with agena-testkit::TestClock. The legacy integer format
/// remains accepted so older builder.scheduler_clock() fixtures keep working.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockControl {
    version: u32,
    pub(crate) now_ms: i64,
    // The scheduler uses wall time; the testkit owns this monotonic timeline.
    #[serde(rename = "elapsed_ms")]
    _elapsed_ms: u64,
    pub(crate) revision: u64,
    pub(crate) scheduler_paused: bool,
}

impl ClockControl {
    fn parse(raw: &str) -> Option<Self> {
        let state = if let Ok(now_ms) = raw.trim().parse::<i64>() {
            Self {
                version: 1,
                now_ms,
                _elapsed_ms: 0,
                revision: 0,
                scheduler_paused: false,
            }
        } else {
            serde_json::from_str(raw).ok()?
        };
        if state.version != 1 || DateTime::<Utc>::from_timestamp_millis(state.now_ms).is_none() {
            return None;
        }
        Some(state)
    }

    pub(crate) fn now(self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.now_ms).expect("validated test clock timestamp")
    }
}

/// Invalid opted-in control files are fixture failures; never silently fall
/// back to system time and hide an incorrect test.
pub(crate) fn control() -> Option<ClockControl> {
    let path = controlled_path()?;
    let raw = std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "test scheduler clock {} cannot be read: {error}",
            path.display()
        )
    });
    Some(ClockControl::parse(&raw).unwrap_or_else(|| {
        panic!(
            "test scheduler clock {} contains an invalid clock control document",
            path.display()
        )
    }))
}

pub fn now() -> DateTime<Utc> {
    control().map_or_else(Utc::now, ClockControl::now)
}

/// Claim ownership follows the real lease/heartbeat domain, not the test's
/// simulated calendar. Jumping virtual business time must not revoke a healthy
/// in-flight delivery before its real heartbeat has had a chance to run.
/// Ordinary product processes already use this system UTC domain.
pub(crate) fn lease_now() -> DateTime<Utc> {
    Utc::now()
}

#[derive(Serialize)]
struct SchedulerObservation {
    revision: u64,
    now_ms: i64,
    scheduler_paused: bool,
    active_deliveries: usize,
    process_id: u32,
}

/// Published only by the scheduler loop after it has applied this control
/// revision at an admission boundary. The caller coalesces unchanged states.
/// Separate atomic files ensure observing a revision never overwrites a clock
/// update concurrently published by the test process.
pub(crate) fn acknowledge(control: ClockControl, active_deliveries: usize) {
    let Some(path) = controlled_path() else {
        return;
    };
    let target = path.with_extension("observed.json");
    let tmp = path.with_extension(format!("{}.observed.swap", uuid::Uuid::new_v4()));
    let observation = SchedulerObservation {
        revision: control.revision,
        now_ms: control.now_ms,
        scheduler_paused: control.scheduler_paused,
        active_deliveries,
        process_id: std::process::id(),
    };
    let data = serde_json::to_vec(&observation).expect("serialize scheduler clock observation");
    let result = std::fs::write(&tmp, data).and_then(|()| std::fs::rename(&tmp, &target));
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        panic!(
            "test scheduler clock observation {} cannot be published: {error}",
            target.display()
        );
    }
}

/// Check for cross-process clock changes promptly, without applying this
/// polling cost to ordinary product processes.
pub(crate) fn reconciliation_tick() -> Option<Duration> {
    controlled_path().map(|_| Duration::from_millis(25))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip_and_invalid_overflow_does_not_wrap() {
        for ms in [0, -1, 1, 1_700_000_000_123, 2_000_000_000_000] {
            let parsed = ClockControl::parse(&ms.to_string()).unwrap();
            assert_eq!(parsed.now().timestamp_millis(), ms);
            assert_eq!(parsed.revision, 0);
            assert!(!parsed.scheduler_paused);
        }
        for raw in [
            "",
            "not-a-date",
            "9999999999999999999999",
            "9223372036854775807",
        ] {
            assert!(ClockControl::parse(raw).is_none());
        }
    }

    #[test]
    fn versioned_clock_requires_a_valid_complete_state() {
        let valid = serde_json::json!({
            "version": 1, "now_ms": 1_700_000_000_123_i64,
            "elapsed_ms": 10_800_000, "revision": 42, "scheduler_paused": true,
        });
        let parsed = ClockControl::parse(&valid.to_string()).unwrap();
        assert_eq!(parsed.now().timestamp_millis(), 1_700_000_000_123);
        assert_eq!(parsed.revision, 42);
        assert!(parsed.scheduler_paused);
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("elapsed_ms", serde_json::json!(-1)),
            ("now_ms", serde_json::json!(i64::MAX)),
            ("scheduler_paused", serde_json::json!("false")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(
                ClockControl::parse(&invalid.to_string()).is_none(),
                "accepted invalid {field}"
            );
        }
        let mut incomplete = valid;
        incomplete.as_object_mut().unwrap().remove("revision");
        assert!(ClockControl::parse(&incomplete.to_string()).is_none());
    }
}
