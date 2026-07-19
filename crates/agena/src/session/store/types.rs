use super::ProcessorPartIdAllocator;

#[derive(Debug, Default)]
pub(crate) struct GlobalIdAllocator {
    pub(crate) initialized: bool,
    pub(crate) next_message_id: i64,
    pub(crate) next_part_id: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionExportMeta {
    /// Original session id at export time. Used for audit / cross-machine
    /// correlation; the new session always gets a fresh auto-increment id.
    pub(crate) source_session_id: i64,
    pub(crate) title: String,
    pub(crate) source_parent_id: Option<i64>,
    pub(crate) source_relation_kind: crate::session::SessionRelationKind,
    pub(crate) source_cutoff_seq_global: Option<i64>,
    pub(crate) source_message_id: Option<i64>,
    pub(crate) created_at_ms: i64,
    pub(crate) updated_at_ms: i64,
    pub(crate) runtime_state: crate::session::SessionRuntimeState,
    /// Filesystem path of the source workspace at export time, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
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
