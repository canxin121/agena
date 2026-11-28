//! Closing records outlive a plugin declaration and the runtime snapshot that
//! owned its connection.

use std::sync::{Mutex, Weak};

use agena_mcp_client::McpManagedConnection;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, watch};

use super::*;

#[derive(Clone)]
pub(super) struct ManagedIdentity {
    pub(super) id: String,
    pub(super) owner: PluginKey,
    pub(super) display_name: String,
    pub(super) qualified_name: String,
    pub(super) workspace: std::path::PathBuf,
    pub(super) session: i64,
}

struct Record {
    identity: ManagedIdentity,
    generation: u64,
    transport: Option<Arc<McpManagedConnection>>,
    failure: Option<Value>,
}

#[derive(Default)]
pub(in crate::runtime) struct CleanupLedger {
    records: Mutex<BTreeMap<ServerKey, Record>>,
    operations: Mutex<BTreeMap<ServerKey, Weak<AsyncMutex<()>>>>,
    revisions: watch::Sender<u64>,
}

type ServerKey = (i64, String);

impl CleanupLedger {
    pub(super) async fn lock_server(&self, session: i64, id: &str) -> OwnedMutexGuard<()> {
        let operation = {
            let mut operations = self
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            operations.retain(|_, lock| lock.strong_count() != 0);
            let slot = operations.entry((session, id.to_owned())).or_default();
            slot.upgrade().unwrap_or_else(|| {
                let operation = Arc::new(AsyncMutex::new(()));
                *slot = Arc::downgrade(&operation);
                operation
            })
        };
        operation.lock_owned().await
    }

    pub(super) fn revision(&self) -> u64 {
        *self.revisions.borrow()
    }

    pub(super) fn watch(&self) -> watch::Receiver<u64> {
        self.revisions.subscribe()
    }

    fn changed(&self) {
        self.revisions.send_modify(|revision| *revision += 1);
    }

    pub(super) fn check_replacement(&self, session: i64, id: &str, generation: u64) -> Result<()> {
        let records = self
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(record) = records.get(&(session, id.to_owned())) else {
            return Ok(());
        };
        if record.failure.is_some()
            || record
                .transport
                .as_ref()
                .is_some_and(|scope| scope.close_result() == Some(false))
        {
            return Err(host_unavailable(
                "the previous MCP connection did not confirm closure",
            ));
        }
        if record.transport.as_ref().is_some_and(|scope| {
            (record.generation != generation || scope.cancellation().is_cancelled())
                && scope.close_result().is_none()
        }) {
            return Err(host_unavailable(
                "the previous MCP connection is still closing",
            ));
        }
        Ok(())
    }

    pub(super) fn track(
        self: &Arc<Self>,
        identity: ManagedIdentity,
        generation: u64,
        scope: Arc<McpManagedConnection>,
    ) {
        let key = (identity.session, identity.id.clone());
        {
            let mut records = self
                .records
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let failure = records.remove(&key).and_then(|record| record.failure);
            records.insert(
                key.clone(),
                Record {
                    identity,
                    generation,
                    transport: Some(scope.clone()),
                    failure,
                },
            );
        }
        let ledger = Arc::downgrade(self);
        tokio::spawn(async move {
            let confirmed = scope.wait_closed().await;
            let Some(ledger) = ledger.upgrade() else {
                return;
            };
            let mut records = ledger
                .records
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(record) = records.get_mut(&key).filter(|record| {
                record
                    .transport
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &scope))
            }) else {
                return;
            };
            record.transport = None;
            if !confirmed {
                merge_failure(record, json!({ "closeConfirmed": false }));
            }
            if record.failure.is_none() {
                records.remove(&key);
            }
            ledger.changed();
        });
    }

    pub(super) fn probe_failure(&self, identity: ManagedIdentity, generation: u64, result: &Value) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = records
            .entry((identity.session, identity.id.clone()))
            .or_insert_with(|| Record {
                identity,
                generation,
                transport: None,
                failure: None,
            });
        let mut closure = json!({ "closeConfirmed": result["closeConfirmed"] });
        if let Some(error) = result.get("cleanupError") {
            closure["cleanupError"] = error.clone();
        }
        merge_failure(record, closure);
        self.changed();
    }

    pub(super) fn rows(
        &self,
        executor: &ToolExecutor,
        session: i64,
        generation: u64,
    ) -> Vec<(ManagedIdentity, Value)> {
        let records = self
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        records.values().filter_map(|record| {
            let identity = &record.identity;
            if identity.session != session || identity.workspace != executor.workspace_root()
                || !executor.plugin_is_enabled(&identity.owner)
            {
                return None;
            }
            let result = match (&record.failure, record.transport.as_ref().map(|scope| scope.close_result())) {
                (Some(failure), _) => json!({ "state": "failed", "reason": "cleanup", "result": failure }),
                (None, Some(Some(false))) => json!({ "state": "failed", "reason": "cleanup", "result": { "closeConfirmed": false } }),
                (None, Some(None)) if record.generation != generation => json!({ "state": "closing" }),
                (None, _) => return None,
            };
            Some((identity.clone(), result))
        }).collect()
    }
}

fn merge_failure(record: &mut Record, closure: Value) {
    if let Some(first) = &mut record.failure {
        if closure["closeConfirmed"] == false {
            first["closeConfirmed"] = Value::Bool(false);
        }
        if first.get("cleanupError").is_none()
            && let Some(error) = closure.get("cleanupError")
        {
            first["cleanupError"] = error.clone();
        }
    } else {
        record.failure = Some(closure);
    }
}
