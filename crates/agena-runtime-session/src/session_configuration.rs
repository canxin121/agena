use agena_domain::{ExecutionSelection, PermissionConfig, SessionAutoCompactionConfig};

/// Configuration consumed by the concrete session manager.
#[derive(Debug, Clone)]
/// Configuration of the runtime session manager.
pub struct RuntimeSessionManagerConfig {
    pub default_selection: ExecutionSelection,
    pub auxiliary_selection: ExecutionSelection,
    pub auto_title: agena_runtime_config::SessionAutoTitleConfig,
    pub permission: PermissionConfig,
    pub auto_compaction: SessionAutoCompactionConfig,
    pub max_concurrent_tools: usize,
    /// Cap on model turns within one stable run. `None` falls back to
    /// `DEFAULT_MAX_MODEL_TURNS` (500) in the supervised kernel run.
    pub max_turns: Option<usize>,
}

impl Default for RuntimeSessionManagerConfig {
    fn default() -> Self {
        Self {
            default_selection: Default::default(),
            auxiliary_selection: Default::default(),
            auto_title: Default::default(),
            permission: Default::default(),
            auto_compaction: Default::default(),
            max_concurrent_tools: 32,
            // Same default as `DEFAULT_MAX_MODEL_TURNS` in
            // the supervised kernel run (mirrors gemini's MAX_TURNS).
            max_turns: Some(500),
        }
    }
}
