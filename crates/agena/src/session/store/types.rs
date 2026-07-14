use super::ProcessorPartIdAllocator;

#[derive(Debug, Default)]
pub(crate) struct GlobalIdAllocator {
    pub(crate) initialized: bool,
    pub(crate) next_message_id: i64,
    pub(crate) next_part_id: i64,
}

/// Wire-format version for [`SessionExportMeta`]. Bumped whenever the meta
/// shape or replay semantics change. Schema 3 is the first export format with
/// typed execution identities, terminal assistant-message events, and
/// source delegated-task provenance. Import still materializes an independent
/// root session and strips parent-bound lifecycle/permission state.
pub(crate) const SESSION_EXPORT_SCHEMA: u32 = 4;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct SessionExportMeta {
    pub(crate) schema: u32,
    /// Original session id at export time. Used for audit / cross-machine
    /// correlation; the new session always gets a fresh auto-increment id.
    #[serde(default)]
    pub(crate) source_session_id: i64,
    pub(crate) parent_id: Option<i64>,
    pub(crate) depth: i64,
    pub(crate) root_id: i64,
    pub(crate) title: String,
    pub(crate) is_subagent: bool,
    pub(crate) task_id: Option<String>,
    pub(crate) created_at_ms: i64,
    pub(crate) updated_at_ms: i64,
    #[serde(default)]
    pub(crate) runtime_state: crate::session::SessionRuntimeState,
    /// Filesystem path of the source workspace at export time. Optional —
    /// empty when exporter cannot resolve a path or for schema=1 bundles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_workspace_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReservedMessageIds {
    pub(crate) message_id: i64,
    pub(crate) part_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReservedProcessorIds {
    pub(crate) message_id: i64,
    pub(crate) part_ids: ProcessorPartIdAllocator,
}
