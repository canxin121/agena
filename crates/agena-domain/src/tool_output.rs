use serde::{Deserialize, Serialize};

use crate::{StructuredObject, StructuredValue};

/// A managed artifact emitted by a tool or provider-native operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolManagedOutput {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl ToolManagedOutput {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            size_bytes: None,
            media_type: None,
        }
    }
}

/// Structured output returned by a tool or provider-native operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ToolOutput {
    #[serde(default, skip_serializing_if = "StructuredObject::is_empty")]
    pub payload: StructuredObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    non_object_payload: Option<StructuredValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_outputs: Vec<ToolManagedOutput>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl ToolOutput {
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
            && self.non_object_payload.is_none()
            && self.managed_outputs.is_empty()
            && !self.truncated
    }

    pub fn from_json_payload(payload: Option<&serde_json::Value>) -> Result<Self, String> {
        match payload {
            None | Some(serde_json::Value::Null) => Ok(Self::default()),
            Some(value @ serde_json::Value::Object(_)) => {
                Ok(Self::from(StructuredObject::try_from(value.clone())?))
            }
            Some(value) => Ok(Self {
                non_object_payload: Some(StructuredValue::try_from(value.clone())?),
                ..Self::default()
            }),
        }
    }

    pub fn to_json_payload(&self) -> Option<serde_json::Value> {
        self.non_object_payload
            .as_ref()
            .map(|value| value.clone().into())
            .or_else(|| (!self.payload.is_empty()).then(|| self.payload.clone().into()))
    }

    pub fn mark_truncated(&mut self, path: impl Into<String>) {
        self.managed_outputs.push(ToolManagedOutput::new(path));
        self.truncated = true;
    }

    pub fn is_model_truncated(&self) -> bool {
        self.truncated || !self.managed_outputs.is_empty()
    }
}

impl From<StructuredObject> for ToolOutput {
    fn from(payload: StructuredObject) -> Self {
        Self {
            payload,
            ..Self::default()
        }
    }
}
