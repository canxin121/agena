//! Stable plugin and tool identifiers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::PluginError;
pub use agena_plugin_contracts::PluginIdentityError as PluginKeyParseError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identifier of a plugin.
pub struct PluginKey {
    namespace: KeySegment,
    name: KeySegment,
}

impl PluginKey {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, PluginKeyParseError> {
        let namespace = namespace.into();
        let name = name.into();
        let (namespace, name) =
            agena_plugin_contracts::normalize_plugin_identity_parts(&namespace, &name)?;
        Ok(Self {
            namespace: KeySegment(namespace),
            name: KeySegment(name),
        })
    }

    pub fn namespace(&self) -> &str {
        self.namespace.0.as_str()
    }

    pub fn name(&self) -> &str {
        self.name.0.as_str()
    }
}

impl fmt::Display for PluginKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.namespace(), self.name())
    }
}

impl FromStr for PluginKey {
    type Err = PluginKeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (namespace, name) = agena_plugin_contracts::normalize_plugin_identity(s)?;
        Ok(Self {
            namespace: KeySegment(namespace),
            name: KeySegment(name),
        })
    }
}

impl Serialize for PluginKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PluginKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identifier of a tool.
pub struct ToolKey {
    plugin: PluginKey,
    name: ToolName,
}

impl ToolKey {
    pub fn new(plugin: PluginKey, name: impl Into<String>) -> Result<Self, ToolKeyParseError> {
        Ok(Self {
            plugin,
            name: ToolName::new(name.into())?,
        })
    }

    pub fn plugin(&self) -> &PluginKey {
        &self.plugin
    }

    pub fn name(&self) -> &str {
        self.name.0.as_str()
    }
}

impl fmt::Display for ToolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.plugin, self.name())
    }
}

impl FromStr for ToolKey {
    type Err = ToolKeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        let mut parts = trimmed.splitn(3, '.');
        let namespace = parts
            .next()
            .ok_or_else(|| ToolKeyParseError::MissingPlugin(trimmed.to_string()))?;
        let plugin_name = parts
            .next()
            .ok_or_else(|| ToolKeyParseError::MissingPlugin(trimmed.to_string()))?;
        let tool_name = parts
            .next()
            .ok_or_else(|| ToolKeyParseError::MissingTool(trimmed.to_string()))?;
        if tool_name.is_empty() {
            return Err(ToolKeyParseError::MissingTool(trimmed.to_string()));
        }
        let plugin = PluginKey::new(namespace, plugin_name).map_err(ToolKeyParseError::Plugin)?;
        Self::new(plugin, tool_name)
    }
}

impl Serialize for ToolKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ToolKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct KeySegment(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ToolName(String);

impl ToolName {
    fn new(value: String) -> Result<Self, ToolKeyParseError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ToolKeyParseError::MissingTool(value));
        }
        Ok(Self(trimmed.to_string()))
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
/// Error parsing a tool key.
pub enum ToolKeyParseError {
    #[error("tool key `{0}` must include `namespace.plugin`")]
    MissingPlugin(String),
    #[error("tool key `{0}` must include a tool name")]
    MissingTool(String),
    #[error("{0}")]
    Plugin(#[from] PluginKeyParseError),
    #[error("invalid tool name `{value}`: {reason}")]
    InvalidToolName { value: String, reason: String },
}

impl From<PluginKeyParseError> for PluginError {
    fn from(value: PluginKeyParseError) -> Self {
        PluginError::invalid_params(value.to_string())
    }
}

impl From<ToolKeyParseError> for PluginError {
    fn from(value: ToolKeyParseError) -> Self {
        PluginError::invalid_params(value.to_string())
    }
}

/// Edit distance for name suggestions after trimming and ASCII case folding.
pub fn normalized_name_distance(left: &str, right: &str) -> usize {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left == right {
        return 0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let replace = prev[j] + usize::from(left_ch != right_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

#[cfg(test)]
mod name_distance_tests {
    use super::normalized_name_distance;

    #[test]
    fn compares_normalized_names_without_changing_unicode_identity() {
        assert_eq!(normalized_name_distance(" FS.Read ", "fs.read"), 0);
        assert_eq!(normalized_name_distance("", "界"), 1);
        assert_eq!(normalized_name_distance("文件", "文档"), 1);
        assert_eq!(normalized_name_distance("read", "raed"), 2);
        assert_eq!(normalized_name_distance("É", "é"), 1);
    }
}
