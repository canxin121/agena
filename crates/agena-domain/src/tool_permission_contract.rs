//! Authority-bearing tool permission contract.
//!
//! This is the base permission contract for a tool: what the tool may touch
//! (declared paths and networks) and how it behaves (shell, interactive,
//! read-only, task). The permission engine consumes this directly; tool tags
//! are metadata for discovery/UI and never carry authority.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ToolPermissionContract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_paths: Vec<InputPathSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_networks: Vec<InputNetworkSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_access: Vec<PathAccessSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network_access: Vec<NetworkAccessSpec>,
    /// Executes arbitrary shell commands. Bash pattern policy applies and the
    /// tool is never eligible for the read-only fast path.
    #[serde(default)]
    pub shell: bool,
    /// Requires explicit user confirmation before every invocation.
    #[serde(default)]
    pub interactive: bool,
    /// Declared read-only: the tool only reads. The permission engine still
    /// cross-checks the contract (no shell, network, or write access) before
    /// auto-approving it.
    #[serde(default)]
    pub read_only: bool,
    /// Long-running autonomous task tool; excluded from no-task model
    /// profiles.
    #[serde(default)]
    pub task: bool,
    /// Mutates persistent state (writes, sessions, config that is not a
    /// path/network declaration). The permission engine treats `mutating &&
    /// shell` / `mutating && path.write` as the same consumer of the
    /// contract; this is a declaration, never a tag.
    #[serde(default)]
    pub mutating: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Read,
    Write,
}

/// Single declarative path extraction rule. `jsonpath` is a subset:
/// dot-paths (`$.path`, `$.files[*].path`). The host extracts each match
/// from the tool input JSON, classifies it under [`PathKind`], and runs it
/// through the permission auditor before the tool body executes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPathSpec {
    pub jsonpath: String,
    pub kind: PathKind,
    /// Value used when `jsonpath` has no matches. This is useful for inputs
    /// whose omitted field has a meaningful permission target, such as the
    /// workspace root represented by an empty path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// If true, missing matches are silently ignored instead of erroring.
    #[serde(default)]
    pub optional: bool,
}

/// Single declarative network extraction rule. `jsonpath` uses the same subset
/// as [`InputPathSpec`]. Each match must resolve to a string URL, host, or
/// host:port target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputNetworkSpec {
    pub jsonpath: String,
    /// Value used when `jsonpath` has no matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// If true, missing matches are silently ignored instead of erroring.
    #[serde(default)]
    pub optional: bool,
}

/// One static filesystem target used by a plugin tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathAccessSpec {
    pub path: String,
    pub kind: PathKind,
}

/// One static outbound network target used by a plugin tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkAccessSpec {
    pub target: String,
}
