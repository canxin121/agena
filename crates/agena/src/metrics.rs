//! Lightweight in-process metrics counters.
//!
//! These are not a real meter provider — they are atomic counters that
//! `agena-api-server`'s `/metrics` endpoint reads to expose Prometheus
//! samples. When `agena-otel` grows a meter API this module should be
//! deleted in favour of proper instruments.
//!
//! Callers bump the counters at the points where the events happen
//! (provider call, tool execution); the HTTP layer formats them.

use std::sync::atomic::{AtomicU64, Ordering};

/// Total provider calls observed (both `complete` and `complete_stream`).
pub static PROVIDER_CALLS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// Provider calls that returned an error.
pub static PROVIDER_CALLS_ERROR: AtomicU64 = AtomicU64::new(0);
/// Provider streaming calls observed.
pub static PROVIDER_STREAM_TOTAL: AtomicU64 = AtomicU64::new(0);
/// Tool invocations observed.
pub static TOOL_EXECUTIONS_TOTAL: AtomicU64 = AtomicU64::new(0);
/// Tool invocations that failed.
pub static TOOL_EXECUTIONS_ERROR: AtomicU64 = AtomicU64::new(0);
/// Sessions currently being processed (incremented on submit, decremented
/// on completion).
pub static SESSION_ACTIVE: AtomicU64 = AtomicU64::new(0);

/// Convenience helpers so call sites stay short.
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
    // saturating subtract — never underflow on bookkeeping bugs.
    let _ = SESSION_ACTIVE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        Some(v.saturating_sub(1))
    });
}

/// Snapshot of every counter, for the Prometheus formatter to read.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricsSnapshot {
    pub provider_calls_total: u64,
    pub provider_calls_error: u64,
    pub provider_stream_total: u64,
    pub tool_executions_total: u64,
    pub tool_executions_error: u64,
    pub session_active: u64,
}

pub fn snapshot() -> MetricsSnapshot {
    MetricsSnapshot {
        provider_calls_total: PROVIDER_CALLS_TOTAL.load(Ordering::Relaxed),
        provider_calls_error: PROVIDER_CALLS_ERROR.load(Ordering::Relaxed),
        provider_stream_total: PROVIDER_STREAM_TOTAL.load(Ordering::Relaxed),
        tool_executions_total: TOOL_EXECUTIONS_TOTAL.load(Ordering::Relaxed),
        tool_executions_error: TOOL_EXECUTIONS_ERROR.load(Ordering::Relaxed),
        session_active: SESSION_ACTIVE.load(Ordering::Relaxed),
    }
}
