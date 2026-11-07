//! Versioned cross-process clock state. Keep the wire format compatible with
//! agena-scheduler::clock; integer-only files from ClockDriver remain readable.

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockState {
    pub version: u32,
    pub now_ms: i64,
    pub elapsed_ms: u64,
    pub revision: u64,
    pub scheduler_paused: bool,
}

impl ClockState {
    pub fn parse(raw: &str) -> Result<Self> {
        let state = if let Ok(now_ms) = raw.trim().parse::<i64>() {
            Self {
                version: 1,
                now_ms,
                elapsed_ms: 0,
                revision: 0,
                scheduler_paused: false,
            }
        } else {
            serde_json::from_str(raw).context("invalid test clock control document")?
        };
        ensure!(
            state.version == 1,
            "unsupported test clock version {}",
            state.version
        );
        state.now()?;
        Ok(state)
    }

    pub fn now(&self) -> Result<DateTime<Utc>> {
        DateTime::from_timestamp_millis(self.now_ms)
            .context("test clock timestamp is not representable")
    }

    pub fn revise(&mut self) -> Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .context("test clock revision overflow")?;
        Ok(())
    }
}

pub(super) fn exact_millis(now: DateTime<Utc>) -> Result<i64> {
    let millis = now.timestamp_millis();
    ensure!(
        DateTime::from_timestamp_millis(millis) == Some(now),
        "test clock has millisecond precision; sub-millisecond instants and leap seconds are unsupported"
    );
    Ok(millis)
}

pub(super) fn duration_millis(duration: std::time::Duration) -> Result<u64> {
    ensure!(
        duration.subsec_nanos().is_multiple_of(1_000_000),
        "test clock has millisecond precision; sub-millisecond durations are unsupported"
    );
    duration
        .as_millis()
        .try_into()
        .context("test clock duration overflow")
}
