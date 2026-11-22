//! Runtime-owned process state for a concrete composition adapter.

use std::{path::PathBuf, sync::Arc};

use crate::RuntimeControlState;

/// Long-lived state retained by a composed runtime process.
///
/// Loader, request, snapshot, and error remain generic so concrete schema and
/// adapter implementations can be injected without making Runtime depend on
/// their owning crate.
pub(crate) struct RuntimeProcessState<Loader, Request, Snapshot, Error> {
    pub(crate) loader: Loader,
    pub(crate) load_request: Request,
    pub(crate) workspace_root: PathBuf,
    pub(crate) session_kernel: Option<Arc<agena_storage_sqlite::SqliteKernel>>,
    /// Shared instance connection retained for scheduler snapshot reloads.
    pub(crate) scheduler_database: Option<crate::scheduler_composition::SchedulerDatabase>,
    pub(crate) control_state: RuntimeControlState<Snapshot, Error>,
}

impl<Loader, Request, Snapshot, Error> RuntimeProcessState<Loader, Request, Snapshot, Error>
where
    Error: Send + Sync + 'static,
{
    pub(crate) fn new(
        loader: Loader,
        load_request: Request,
        workspace_root: PathBuf,
        session_kernel: Option<Arc<agena_storage_sqlite::SqliteKernel>>,
        scheduler_database: Option<crate::scheduler_composition::SchedulerDatabase>,
        control_state: RuntimeControlState<Snapshot, Error>,
    ) -> Self {
        Self {
            loader,
            load_request,
            workspace_root,
            session_kernel,
            scheduler_database,
            control_state,
        }
    }
}
