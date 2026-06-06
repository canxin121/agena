use serde::{Deserialize, Serialize};

use crate::manifest::PathKind;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Prompt,
}

/// One filesystem path that a tool intends to read or write. Returned by
/// [`crate::Plugin::permission_paths`] for paths that cannot be expressed as
/// declarative `InputPathSpec` JSONPath rules (e.g. paths derived from a
/// patch body or shell command parsing).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathRequest {
    pub path: String,
    pub kind: PathKind,
}

impl PathRequest {
    pub fn read(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: PathKind::Read,
        }
    }

    pub fn write(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: PathKind::Write,
        }
    }
}

pub trait IntoPathRequests {
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>>;
}

impl IntoPathRequests for Vec<PathRequest> {
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        Ok(self)
    }
}

impl IntoPathRequests for PathRequest {
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        Ok(vec![self])
    }
}

impl<T, E> IntoPathRequests for std::result::Result<T, E>
where
    T: IntoPathRequests,
    E: Into<crate::PluginError>,
{
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        match self {
            Ok(value) => value.into_path_requests(),
            Err(err) => Err(err.into()),
        }
    }
}

/// One outbound network target that a tool intends to connect to. Returned by
/// [`crate::Plugin::permission_networks`] for targets that cannot be expressed
/// as declarative `InputNetworkSpec` JSONPath rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkRequest {
    pub target: String,
}

impl NetworkRequest {
    pub fn connect(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
        }
    }
}

pub trait IntoNetworkRequests {
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>>;
}

impl IntoNetworkRequests for Vec<NetworkRequest> {
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        Ok(self)
    }
}

impl IntoNetworkRequests for NetworkRequest {
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        Ok(vec![self])
    }
}

impl<T, E> IntoNetworkRequests for std::result::Result<T, E>
where
    T: IntoNetworkRequests,
    E: Into<crate::PluginError>,
{
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        match self {
            Ok(value) => value.into_network_requests(),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAskInput {
    pub session_id: i64,
    pub action: String,
    #[serde(default)]
    pub subject: serde_json::Value,
    pub default_decision: PermissionDecision,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRiskLevel {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionAdvice {
    pub decision: PermissionDecision,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub risk: PermissionRiskLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PermissionAskDecision {
    Decide(PermissionDecision),
    Advise(PermissionAdvice),
    Defer,
}
