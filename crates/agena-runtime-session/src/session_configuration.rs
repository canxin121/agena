use agena_domain::{
    ExecutionSelection, PermissionConfig, SessionAutoCompactionConfig, SessionCacheLimits,
};

use crate::SessionCachePolicy;

/// Configuration consumed by the concrete session manager.
#[derive(Debug, Clone)]
pub struct RuntimeSessionManagerConfig {
    pub default_selection: ExecutionSelection,
    pub permission: PermissionConfig,
    pub auto_compaction: SessionAutoCompactionConfig,
    pub cache_limits: SessionCacheLimits,
    pub max_concurrent_tools: usize,
}

impl Default for RuntimeSessionManagerConfig {
    fn default() -> Self {
        Self {
            default_selection: Default::default(),
            permission: Default::default(),
            auto_compaction: Default::default(),
            cache_limits: Default::default(),
            max_concurrent_tools: 32,
        }
    }
}

impl RuntimeSessionManagerConfig {
    pub fn cache_policy(&self) -> SessionCachePolicy {
        SessionCachePolicy::from_limits(self.cache_limits)
    }
}
