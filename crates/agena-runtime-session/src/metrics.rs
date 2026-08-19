//! Lightweight process-local runtime metrics shared by composition adapters.

use portable_atomic::AtomicU64;
use std::sync::atomic::Ordering;

pub static PROVIDER_CALLS_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static PROVIDER_CALLS_ERROR: AtomicU64 = AtomicU64::new(0);
pub static PROVIDER_STREAM_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static TOOL_EXECUTIONS_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static TOOL_EXECUTIONS_ERROR: AtomicU64 = AtomicU64::new(0);
pub static SESSION_ACTIVE: AtomicU64 = AtomicU64::new(0);

pub fn record_provider_call(success: bool) {
    PROVIDER_CALLS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if !success {
        PROVIDER_CALLS_ERROR.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn record_provider_stream() {
    PROVIDER_STREAM_TOTAL.fetch_add(1, Ordering::Relaxed);
}
pub fn record_tool_execution(success: bool) {
    TOOL_EXECUTIONS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if !success {
        TOOL_EXECUTIONS_ERROR.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn session_started() {
    SESSION_ACTIVE.fetch_add(1, Ordering::Relaxed);
}
pub fn session_finished() {
    let _ = SESSION_ACTIVE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_sub(1))
    });
}

#[derive(Debug, Clone, Copy, Default)]
/// Snapshot of runtime metrics.
pub struct RuntimeMetricsSnapshot {
    pub provider_calls_total: u64,
    pub provider_calls_error: u64,
    pub provider_stream_total: u64,
    pub tool_executions_total: u64,
    pub tool_executions_error: u64,
    pub session_active: u64,
}

pub fn runtime_metrics_snapshot() -> RuntimeMetricsSnapshot {
    RuntimeMetricsSnapshot {
        provider_calls_total: PROVIDER_CALLS_TOTAL.load(Ordering::Relaxed),
        provider_calls_error: PROVIDER_CALLS_ERROR.load(Ordering::Relaxed),
        provider_stream_total: PROVIDER_STREAM_TOTAL.load(Ordering::Relaxed),
        tool_executions_total: TOOL_EXECUTIONS_TOTAL.load(Ordering::Relaxed),
        tool_executions_error: TOOL_EXECUTIONS_ERROR.load(Ordering::Relaxed),
        session_active: SESSION_ACTIVE.load(Ordering::Relaxed),
    }
}
