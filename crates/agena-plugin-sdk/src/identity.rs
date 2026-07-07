use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::PluginError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginKey {
    namespace: KeySegment,
    name: KeySegment,
}

impl PluginKey {
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, PluginKeyParseError> {
        Ok(Self {
            namespace: KeySegment::new(namespace.into(), "plugin namespace")?,
            name: KeySegment::new(name.into(), "plugin name")?,
        })
    }

    pub fn parse(value: &str) -> Result<Self, PluginKeyParseError> {
        let trimmed = value.trim();
        let Some((namespace, name)) = trimmed.split_once('.') else {
            return Err(PluginKeyParseError::MissingSeparator(trimmed.to_string()));
        };
        if name.contains('.') {
            return Err(PluginKeyParseError::InvalidComponent {
                label: "plugin name",
                value: name.to_string(),
                reason: "must not contain `.`".to_string(),
            });
        }
        Self::new(namespace, name)
    }

    pub fn namespace(&self) -> &str {
        self.namespace.as_str()
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn to_model_string(&self) -> String {
        format!("{}.{}", self.namespace(), self.name())
    }
}

impl fmt::Display for PluginKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_model_string().as_str())
    }
}

impl FromStr for PluginKey {
    type Err = PluginKeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for PluginKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_model_string().as_str())
    }
}

impl<'de> Deserialize<'de> for PluginKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value.as_str()).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    pub fn parse_model_name(value: &str) -> Result<Self, ToolKeyParseError> {
        let trimmed = value.trim();
        let mut parts = trimmed.split('.');
        let namespace = parts
            .next()
            .ok_or_else(|| ToolKeyParseError::MissingPlugin(trimmed.to_string()))?;
        let plugin_name = parts
            .next()
            .ok_or_else(|| ToolKeyParseError::MissingPlugin(trimmed.to_string()))?;
        let tool_name = parts.collect::<Vec<_>>().join(".");
        if tool_name.is_empty() {
            return Err(ToolKeyParseError::MissingTool(trimmed.to_string()));
        }
        let plugin = PluginKey::new(namespace, plugin_name).map_err(ToolKeyParseError::Plugin)?;
        Self::new(plugin, tool_name)
    }

    pub fn plugin(&self) -> &PluginKey {
        &self.plugin
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn to_model_string(&self) -> String {
        format!("{}.{}", self.plugin, self.name())
    }

    pub fn to_provider_safe_string(&self) -> String {
        let trimmed = self.to_model_string();
        let mut out = String::with_capacity(trimmed.len());
        let mut previous_was_separator = false;

        for ch in trimmed.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                out.push(ch);
                previous_was_separator = false;
            } else if !previous_was_separator {
                out.push('_');
                previous_was_separator = true;
            }
        }

        while out.ends_with('_') {
            out.pop();
        }
        while out.starts_with('_') {
            out.remove(0);
        }
        if out.is_empty() {
            out.push_str("tool");
        }
        if out
            .bytes()
            .next()
            .is_some_and(|byte| !byte.is_ascii_alphabetic() && byte != b'_')
        {
            out.insert(0, '_');
        }
        out
    }
}

impl fmt::Display for ToolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_model_string().as_str())
    }
}

impl Serialize for ToolKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_model_string().as_str())
    }
}

impl<'de> Deserialize<'de> for ToolKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse_model_name(value.as_str()).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct KeySegment(String);

impl KeySegment {
    fn new(value: String, label: &'static str) -> Result<Self, PluginKeyParseError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(PluginKeyParseError::InvalidComponent {
                label,
                value,
                reason: "cannot be empty".to_string(),
            });
        }
        if trimmed.contains('.') {
            return Err(PluginKeyParseError::InvalidComponent {
                label,
                value,
                reason: "must not contain `.`".to_string(),
            });
        }
        if trimmed.contains('/') {
            return Err(PluginKeyParseError::InvalidComponent {
                label,
                value,
                reason: "must not contain `/`".to_string(),
            });
        }
        Ok(Self(trimmed.to_string()))
    }

    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ToolName(String);

impl ToolName {
    fn new(value: String) -> Result<Self, ToolKeyParseError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ToolKeyParseError::MissingTool(value));
        }
        if trimmed.contains('/') {
            return Err(ToolKeyParseError::InvalidToolName {
                value,
                reason: "must not contain `/`".to_string(),
            });
        }
        Ok(Self(trimmed.to_string()))
    }

    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PluginKeyParseError {
    #[error("plugin key `{0}` must use `namespace.plugin` format")]
    MissingSeparator(String),
    #[error("invalid {label} `{value}`: {reason}")]
    InvalidComponent {
        label: &'static str,
        value: String,
        reason: String,
    },
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
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
