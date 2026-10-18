//! Infrastructure diagnostics; these counters confer no session/storage write
//! authority. Commit counters advance only after an acknowledged COMMIT.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

use serde::Serialize;

macro_rules! counters {
    ($($name:ident),+ $(,)?) => {
        #[derive(Default)]
        pub(super) struct Counters {
            $(pub $name: AtomicU64,)+
            read_transaction_duration_us_log2: [AtomicU64; 32],
        }
        #[derive(Debug,Clone,Default,Serialize)]
        pub struct KernelMetricsSnapshot {
            $(pub $name: u64,)+
            /// Bucket i covers completed transactions taking at most 2^i us.
            /// The last bucket also includes durations above 2^31 us.
            pub read_transaction_duration_us_log2: [u64; 32],
        }
        impl KernelDiagnostics {
            pub fn snapshot(&self) -> KernelMetricsSnapshot {
                KernelMetricsSnapshot {
                    $($name: self.0.$name.load(Ordering::Relaxed),)+
                    read_transaction_duration_us_log2: std::array::from_fn(|i| self.0.read_transaction_duration_us_log2[i].load(Ordering::Relaxed)),
                }
            }
        }
    }
}

counters!(
    accepted_text_appends,
    accepted_text_bytes,
    accepted_events,
    command_transactions,
    command_results,
    checkpoint_transactions,
    text_blocks,
    event_blocks,
    event_block_queries,
    event_block_rows,
    event_block_validated_bytes,
    event_page_decoded_records,
    history_queries,
    history_candidate_rows,
    history_entry_rows,
    history_entry_body_bytes,
    owned_tool_queries,
    owned_tool_candidate_rows,
    owned_tool_entry_rows,
    owned_tool_entry_body_bytes,
    owned_tool_admitted_bytes,
    metadata_queries,
    detail_queries,
    activity_queries,
    activity_rows,
    interaction_candidate_rows,
    interaction_body_bytes,
    summary_queries,
    summary_rows,
    prompt_queries,
    prompt_candidate_rows,
    prompt_body_bytes,
    snapshot_queries,
    snapshot_candidate_rows,
    snapshot_body_bytes,
    read_transactions_started,
    read_transactions_completed,
    read_transactions_abandoned,
    read_transaction_active_scopes,
    read_transaction_peak_active_scopes,
    read_transaction_total_completed_ns,
    read_transaction_max_completed_ns,
);

#[derive(Clone, Default)]
pub struct KernelDiagnostics(
    pub(super) Arc<Counters>,
    pub(super) Arc<super::read_cache::ReadCache>,
    pub(super) agena_storage::kernel::ReadBudget,
);

impl KernelDiagnostics {
    /// Call immediately after an explicit read BEGIN is acknowledged. A normal
    /// result must await ROLLBACK before calling `complete`; SQLx's Drop merely
    /// queues rollback, so abandoned scopes never enter the duration histogram.
    /// These counters cover instrumented archive/history/blob/control-snapshot/content-event
    /// read transactions, not implicit single-statement reads or transactions
    /// waiting to begin.
    pub(super) fn read_transaction(&self) -> ReadTransactionMeasurement {
        let started = Instant::now();
        self.0
            .read_transactions_started
            .fetch_add(1, Ordering::Relaxed);
        let active = self
            .0
            .read_transaction_active_scopes
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        self.0
            .read_transaction_peak_active_scopes
            .fetch_max(active, Ordering::Relaxed);
        ReadTransactionMeasurement {
            counters: self.0.clone(),
            started,
            completed: false,
        }
    }

    pub fn read_cache(&self) -> super::read_cache::ReadCacheSnapshot {
        self.1.snapshot()
    }
    pub fn read_budget(&self) -> agena_storage::kernel::ReadBudgetSnapshot {
        self.2.snapshot()
    }
}

/// `active_scopes` counts measurement guards, not SQL transactions whose
/// asynchronous rollback has completed. A nonzero abandoned count means their
/// actual end was not observed; it must not be reported as a completed duration.
pub(super) struct ReadTransactionMeasurement {
    counters: Arc<Counters>,
    started: Instant,
    completed: bool,
}

impl ReadTransactionMeasurement {
    pub(super) fn complete(mut self) {
        let ns = self.started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        let us = ns.div_ceil(1000).max(1);
        let bucket = ((u64::BITS - (us - 1).leading_zeros()) as usize).min(31);
        self.counters.read_transaction_duration_us_log2[bucket].fetch_add(1, Ordering::Relaxed);
        self.counters
            .read_transaction_total_completed_ns
            .fetch_add(ns, Ordering::Relaxed);
        self.counters
            .read_transaction_max_completed_ns
            .fetch_max(ns, Ordering::Relaxed);
        self.counters
            .read_transactions_completed
            .fetch_add(1, Ordering::Relaxed);
        self.completed = true;
    }
}

impl Drop for ReadTransactionMeasurement {
    fn drop(&mut self) {
        if !self.completed {
            self.counters
                .read_transactions_abandoned
                .fetch_add(1, Ordering::Relaxed);
        }
        self.counters
            .read_transaction_active_scopes
            .fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) fn add(counter: &AtomicU64, amount: usize) {
    counter.fetch_add(amount as u64, Ordering::Relaxed);
}
