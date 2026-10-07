use std::time::Duration;

/// Scheduling switches and intervals used by runtime background loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSchedulingPolicy {
    pub reload_enabled: bool,
    pub reload_poll_interval: Duration,
    pub session_gc_enabled: bool,
    pub session_gc_interval: Duration,
    /// Safety reconciliation for external writers/clock changes. Ordinary
    /// scheduling follows exact deadlines and committed-change notifications.
    pub scheduler_reconciliation_interval: Duration,
}

impl Default for RuntimeSchedulingPolicy {
    fn default() -> Self {
        Self {
            reload_enabled: true,
            reload_poll_interval: Duration::from_secs(2),
            session_gc_enabled: true,
            session_gc_interval: Duration::from_secs(30),
            scheduler_reconciliation_interval: Duration::from_secs(60),
        }
    }
}
