//! Presentation state and scheduling policy for an optional terminal status line.
//!
//! The application host executes the configured command because that is a
//! process side effect. This module owns the terminal-facing enablement,
//! refresh cadence, in-flight suppression, and displayed text.

use std::time::{Duration, Instant};

use crate::presentation_config::TuiStatusLineConfig;

/// TUI effect requested when an external status-line command is due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLineEffect {
    None,
    Refresh { command: String },
}

/// Terminal presentation state for one optional, periodically refreshed status
/// line. Command execution and process-derived interpolation remain outside
/// this state machine.
#[derive(Debug, Clone)]
pub struct StatusLinePresentation {
    command: String,
    refresh_interval: Duration,
    next_refresh_at: Instant,
    text: Option<String>,
    refresh_in_flight: bool,
}

impl StatusLinePresentation {
    /// Builds presentation state only when the status-line command is enabled.
    pub fn from_config(config: &TuiStatusLineConfig) -> Option<Self> {
        let command = config.command.as_ref()?.clone();
        Some(Self {
            command,
            refresh_interval: Duration::from_millis(config.refresh_interval_ms),
            next_refresh_at: Instant::now(),
            text: None,
            refresh_in_flight: false,
        })
    }

    /// Reduces one presentation tick into a command-refresh intent. While a
    /// command is in flight, no concurrent refresh is requested.
    pub fn tick(&mut self, now: Instant) -> StatusLineEffect {
        if self.refresh_in_flight || now < self.next_refresh_at {
            return StatusLineEffect::None;
        }

        self.refresh_in_flight = true;
        self.next_refresh_at = now + self.refresh_interval;
        StatusLineEffect::Refresh {
            command: self.command.clone(),
        }
    }

    /// Applies the terminal text emitted by a completed process-side refresh.
    pub fn apply_refresh(&mut self, output: Option<String>) {
        self.refresh_in_flight = false;
        self.text = output;
    }

    /// Returns the last completed status-line text for read-only rendering.
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{StatusLineEffect, StatusLinePresentation};
    use crate::presentation_config::TuiStatusLineConfig;

    #[test]
    fn disabled_config_has_no_presentation_state() {
        assert!(StatusLinePresentation::from_config(&TuiStatusLineConfig::default()).is_none());
    }

    #[test]
    fn tick_requests_one_refresh_until_its_result_is_applied() {
        let mut status_line = StatusLinePresentation::from_config(&TuiStatusLineConfig {
            command: Some("git status --short".to_owned()),
            refresh_interval_ms: 100,
        })
        .expect("configured command must enable the status line");
        let now = Instant::now();

        assert_eq!(
            status_line.tick(now),
            StatusLineEffect::Refresh {
                command: "git status --short".to_owned(),
            }
        );
        assert_eq!(
            status_line.tick(now + Duration::from_secs(1)),
            StatusLineEffect::None
        );

        status_line.apply_refresh(Some("main".to_owned()));
        assert_eq!(status_line.text(), Some("main"));
        assert_eq!(
            status_line.tick(now + Duration::from_millis(99)),
            StatusLineEffect::None
        );
        assert_eq!(
            status_line.tick(now + Duration::from_millis(100)),
            StatusLineEffect::Refresh {
                command: "git status --short".to_owned(),
            }
        );
    }
}
