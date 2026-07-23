use serde::{Deserialize, Serialize};

use crate::StructuredObject;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_outputs: Vec<ToolManagedOutput>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl ToolOutput {
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty() && self.managed_outputs.is_empty() && !self.truncated
    }

    pub fn from_json_payload(payload: Option<&serde_json::Value>) -> Result<Self, String> {
        match payload {
            None | Some(serde_json::Value::Null) => Ok(Self::default()),
            Some(value) => Ok(Self {
                payload: StructuredObject::try_from(value.clone())?,
                managed_outputs: Vec::new(),
                truncated: false,
            }),
        }
    }

    pub fn to_json_payload(&self) -> Option<serde_json::Value> {
        (!self.payload.is_empty()).then(|| serde_json::Value::from(self.payload.clone()))
    }

    pub fn mark_truncated(&mut self, path: impl Into<String>) {
        self.managed_outputs.push(ToolManagedOutput::new(path));
        self.truncated = true;
    }

    pub fn is_model_truncated(&self) -> bool {
        self.truncated || !self.managed_outputs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_payload_and_managed_output_truncation() {
        let payload = serde_json::json!({"result": {"count": 2}});
        let mut output = ToolOutput::from_json_payload(Some(&payload)).expect("structured payload");

        assert_eq!(output.to_json_payload(), Some(payload));
        assert!(!output.is_model_truncated());

        output.mark_truncated("/tmp/tool-output.txt");

        assert!(output.truncated);
        assert!(output.is_model_truncated());
        assert_eq!(output.managed_outputs[0].path, "/tmp/tool-output.txt");
    }
}
