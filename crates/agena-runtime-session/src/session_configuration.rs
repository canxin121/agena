use agena_domain::{
    ExecutionSelection, PermissionConfig, SessionAutoCompactionConfig, SessionCacheLimits,
};

use crate::SessionCachePolicy;

/// Configuration consumed by the concrete session manager.
#[derive(Debug, Clone)]
/// Configuration of the runtime session manager.
pub struct RuntimeSessionManagerConfig {
    pub default_selection: ExecutionSelection,
    pub permission: PermissionConfig,
    pub auto_compaction: SessionAutoCompactionConfig,
    pub cache_limits: SessionCacheLimits,
    pub max_concurrent_tools: usize,
    /// Cap on model turns within one stable run. `None` falls back to
    /// `DEFAULT_MAX_MODEL_TURNS` (500) in `replies_execution.rs`.
    pub max_turns: Option<usize>,
}

impl Default for RuntimeSessionManagerConfig {
    fn default() -> Self {
        Self {
            default_selection: Default::default(),
            permission: Default::default(),
            auto_compaction: Default::default(),
            cache_limits: Default::default(),
            max_concurrent_tools: 32,
            // Same default as `DEFAULT_MAX_MODEL_TURNS` in
            // `replies_execution.rs` (mirrors gemini's MAX_TURNS).
            max_turns: Some(500),
        }
    }
}

impl RuntimeSessionManagerConfig {
    pub fn cache_policy(&self) -> SessionCachePolicy {
        SessionCachePolicy::from_limits(self.cache_limits)
    }
}
