use std::{borrow::Cow, path::Path};

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

pub trait IntoPermissionPath {
    fn into_permission_path(self) -> crate::Result<Option<String>>;
}

impl IntoPermissionPath for String {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self))
    }
}

impl IntoPermissionPath for &String {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.clone()))
    }
}

impl IntoPermissionPath for &str {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.to_string()))
    }
}

impl IntoPermissionPath for Cow<'_, str> {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.into_owned()))
    }
}

impl IntoPermissionPath for &Cow<'_, str> {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.as_ref().to_string()))
    }
}

impl IntoPermissionPath for std::path::PathBuf {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.to_string_lossy().into_owned()))
    }
}

impl IntoPermissionPath for &std::path::PathBuf {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.to_string_lossy().into_owned()))
    }
}

impl IntoPermissionPath for &Path {
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        Ok(Some(self.to_string_lossy().into_owned()))
    }
}

impl<T> IntoPermissionPath for Option<T>
where
    T: IntoPermissionPath,
{
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        match self {
            Some(value) => value.into_permission_path(),
            None => Ok(None),
        }
    }
}

impl<T, E> IntoPermissionPath for std::result::Result<T, E>
where
    T: IntoPermissionPath,
    E: Into<crate::PluginError>,
{
    fn into_permission_path(self) -> crate::Result<Option<String>> {
        self.map_err(Into::into)?.into_permission_path()
    }
}

pub trait IntoPermissionPaths {
    fn into_permission_paths(self) -> crate::Result<Vec<String>>;
}

impl IntoPermissionPaths for () {
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl<T> IntoPermissionPaths for Vec<T>
where
    T: IntoPermissionPath,
{
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        collect_permission_paths(self)
    }
}

impl<T> IntoPermissionPaths for &Vec<T>
where
    T: Clone + IntoPermissionPath,
{
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        collect_permission_paths(self.iter().cloned())
    }
}

impl<T, const N: usize> IntoPermissionPaths for [T; N]
where
    T: IntoPermissionPath,
{
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        collect_permission_paths(self)
    }
}

impl<T> IntoPermissionPaths for &[T]
where
    T: Clone + IntoPermissionPath,
{
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        collect_permission_paths(self.iter().cloned())
    }
}

impl<T> IntoPermissionPaths for Option<T>
where
    T: IntoPermissionPaths,
{
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        self.map(IntoPermissionPaths::into_permission_paths)
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

impl<T, E> IntoPermissionPaths for std::result::Result<T, E>
where
    T: IntoPermissionPaths,
    E: Into<crate::PluginError>,
{
    fn into_permission_paths(self) -> crate::Result<Vec<String>> {
        self.map_err(Into::into)?.into_permission_paths()
    }
}

fn collect_permission_paths<I, T>(paths: I) -> crate::Result<Vec<String>>
where
    I: IntoIterator<Item = T>,
    T: IntoPermissionPath,
{
    let iter = paths.into_iter();
    let (lower, upper) = iter.size_hint();
    let mut output = Vec::with_capacity(upper.unwrap_or(lower));
    for path in iter {
        if let Some(path) = path.into_permission_path()? {
            output.push(path);
        }
    }
    Ok(output)
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

impl IntoPathRequests for () {
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        Ok(Vec::new())
    }
}

impl<T> IntoPathRequests for Option<T>
where
    T: IntoPathRequests,
{
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        self.map(IntoPathRequests::into_path_requests)
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

impl<const N: usize> IntoPathRequests for [PathRequest; N] {
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        Ok(Vec::from(self))
    }
}

impl IntoPathRequests for &[PathRequest] {
    fn into_path_requests(self) -> crate::Result<Vec<PathRequest>> {
        Ok(self.to_vec())
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

pub trait IntoPermissionTarget {
    fn into_permission_target(self) -> crate::Result<Option<String>>;
}

impl IntoPermissionTarget for String {
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        Ok(Some(self))
    }
}

impl IntoPermissionTarget for &String {
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        Ok(Some(self.clone()))
    }
}

impl IntoPermissionTarget for &str {
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        Ok(Some(self.to_string()))
    }
}

impl IntoPermissionTarget for Cow<'_, str> {
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        Ok(Some(self.into_owned()))
    }
}

impl IntoPermissionTarget for &Cow<'_, str> {
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        Ok(Some(self.as_ref().to_string()))
    }
}

impl<T> IntoPermissionTarget for Option<T>
where
    T: IntoPermissionTarget,
{
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        match self {
            Some(value) => value.into_permission_target(),
            None => Ok(None),
        }
    }
}

impl<T, E> IntoPermissionTarget for std::result::Result<T, E>
where
    T: IntoPermissionTarget,
    E: Into<crate::PluginError>,
{
    fn into_permission_target(self) -> crate::Result<Option<String>> {
        self.map_err(Into::into)?.into_permission_target()
    }
}

pub trait IntoPermissionTargets {
    fn into_permission_targets(self) -> crate::Result<Vec<String>>;
}

impl IntoPermissionTargets for () {
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl<T> IntoPermissionTargets for Vec<T>
where
    T: IntoPermissionTarget,
{
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        collect_permission_targets(self)
    }
}

impl<T> IntoPermissionTargets for &Vec<T>
where
    T: Clone + IntoPermissionTarget,
{
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        collect_permission_targets(self.iter().cloned())
    }
}

impl<T, const N: usize> IntoPermissionTargets for [T; N]
where
    T: IntoPermissionTarget,
{
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        collect_permission_targets(self)
    }
}

impl<T> IntoPermissionTargets for &[T]
where
    T: Clone + IntoPermissionTarget,
{
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        collect_permission_targets(self.iter().cloned())
    }
}

impl<T> IntoPermissionTargets for Option<T>
where
    T: IntoPermissionTargets,
{
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        self.map(IntoPermissionTargets::into_permission_targets)
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

impl<T, E> IntoPermissionTargets for std::result::Result<T, E>
where
    T: IntoPermissionTargets,
    E: Into<crate::PluginError>,
{
    fn into_permission_targets(self) -> crate::Result<Vec<String>> {
        self.map_err(Into::into)?.into_permission_targets()
    }
}

fn collect_permission_targets<I, T>(targets: I) -> crate::Result<Vec<String>>
where
    I: IntoIterator<Item = T>,
    T: IntoPermissionTarget,
{
    let iter = targets.into_iter();
    let (lower, upper) = iter.size_hint();
    let mut output = Vec::with_capacity(upper.unwrap_or(lower));
    for target in iter {
        if let Some(target) = target.into_permission_target()? {
            output.push(target);
        }
    }
    Ok(output)
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

impl IntoNetworkRequests for () {
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        Ok(Vec::new())
    }
}

impl<T> IntoNetworkRequests for Option<T>
where
    T: IntoNetworkRequests,
{
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        self.map(IntoNetworkRequests::into_network_requests)
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

impl<const N: usize> IntoNetworkRequests for [NetworkRequest; N] {
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        Ok(Vec::from(self))
    }
}

impl IntoNetworkRequests for &[NetworkRequest] {
    fn into_network_requests(self) -> crate::Result<Vec<NetworkRequest>> {
        Ok(self.to_vec())
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
