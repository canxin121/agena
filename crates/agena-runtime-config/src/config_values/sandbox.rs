use serde::{Deserialize, Serialize};

/// Host-controlled isolation and the optional checks used for direct execution.
/// Resource declarations and command hints are dormant while MXC is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub resource_checks: bool,
    pub command_hints: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            resource_checks: true,
            command_hints: true,
        }
    }
}
