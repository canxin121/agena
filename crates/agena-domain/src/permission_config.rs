//! Declarative aggregate permission configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ModelRef, NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionMode,
    ToolPermissionConfig,
};

/// Stable, serializable permission configuration independent of policy
/// compilation and host-specific tag interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathPermissionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermissionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolPermissionConfig>,
    /// Model used to make an automatic permission decision. The runtime must
    /// fail closed to an interactive `ask` when this reference is absent or
    /// cannot be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_model: Option<ModelRef>,
}

impl PermissionConfig {
    pub fn global_default() -> Self {
        let auto = PermissionMode::Auto;
        Self {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(PermissionMode::Allow),
                    write: Some(auto),
                }),
                external: Some(PathAccessModes {
                    read: Some(auto),
                    write: Some(auto),
                }),
                ..Default::default()
            }),
            network: Some(NetworkPermissionConfig {
                internet: Some(auto),
                private: Some(auto),
                loopback: Some(auto),
                ..Default::default()
            }),
            tools: Some(ToolPermissionConfig {
                default: Some(auto),
                tags: BTreeMap::from([("filesystem_read".to_string(), PermissionMode::Allow)]),
                names: BTreeMap::from([
                    ("agena.web.search".to_string(), PermissionMode::Allow),
                    ("agena.web.fetch".to_string(), PermissionMode::Allow),
                ]),
                ..Default::default()
            }),
            approval_model: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.path
            .as_ref()
            .is_none_or(PathPermissionConfig::is_empty)
            && self
                .network
                .as_ref()
                .is_none_or(NetworkPermissionConfig::is_empty)
            && self
                .tools
                .as_ref()
                .is_none_or(ToolPermissionConfig::is_empty)
            && self.approval_model.is_none()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        merge_path_section(&mut self.path, overlay.path);
        merge_network_section(&mut self.network, overlay.network);
        merge_tool_section(&mut self.tools, overlay.tools);
        if overlay.approval_model.is_some() {
            self.approval_model = overlay.approval_model;
        }
    }

    pub fn merged_with(&self, overlay: &Self) -> Self {
        let mut merged = self.clone();
        merged.merge_from(overlay.clone());
        merged
    }
}

fn merge_path_section(
    current: &mut Option<PathPermissionConfig>,
    overlay: Option<PathPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

fn merge_network_section(
    current: &mut Option<NetworkPermissionConfig>,
    overlay: Option<NetworkPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

fn merge_tool_section(
    current: &mut Option<ToolPermissionConfig>,
    overlay: Option<ToolPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}
