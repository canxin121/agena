//! Declarative per-tool permission rule shapes.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::PermissionMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
/// Per-tool permission rules: a mode or an ordered name-to-mode map.
pub enum ToolPermissionRules {
    Mode(PermissionMode),
    Ordered(IndexMap<String, PermissionMode>),
}
