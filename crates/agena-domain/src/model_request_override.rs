use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Provider request additions selected by a configured model speed mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelSpeedModeRequestOverride {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body_patch: BTreeMap<String, serde_json::Value>,
}

impl ModelSpeedModeRequestOverride {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body_patch.is_empty()
    }

    /// Remove the legacy protocol field from generic JSON body patches. The
    /// typed model capability is serialized only by adapters that support it.
    pub fn remove_legacy_parallel_tool_calls_patch(&mut self) {
        self.body_patch.remove("parallel_tool_calls");
        for value in self.body_patch.values_mut() {
            remove_parallel_tool_calls_keys(value);
        }
    }

    pub fn merged_with(&self, other: &Self) -> Self {
        let mut merged = self.clone();
        for (key, value) in &other.headers {
            merged.headers.insert(key.clone(), value.clone());
        }
        merge_json_patch_maps(&mut merged.body_patch, &other.body_patch);
        merged.remove_legacy_parallel_tool_calls_patch();
        merged
    }
}

fn remove_parallel_tool_calls_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("parallel_tool_calls");
            for value in object.values_mut() {
                remove_parallel_tool_calls_keys(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                remove_parallel_tool_calls_keys(value);
            }
        }
        _ => {}
    }
}

fn merge_json_patch_maps(
    target: &mut BTreeMap<String, serde_json::Value>,
    patch: &BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in patch {
        match target.get_mut(key) {
            Some(current) => merge_json_value(current, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn merge_json_value(current: &mut serde_json::Value, patch: &serde_json::Value) {
    match (current, patch) {
        (serde_json::Value::Object(current), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                match current.get_mut(key) {
                    Some(existing) => merge_json_value(existing, value),
                    None => {
                        current.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (current, patch) => *current = patch.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::ModelSpeedModeRequestOverride;

    #[test]
    fn overrides_merge_headers_and_nested_json_patches() {
        let base = ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([("x-mode".to_owned(), "base".to_owned())]),
            body_patch: BTreeMap::from([("options".to_owned(), serde_json::json!({"a": 1}))]),
        };
        let patch = ModelSpeedModeRequestOverride {
            headers: BTreeMap::from([("x-mode".to_owned(), "fast".to_owned())]),
            body_patch: BTreeMap::from([(
                "options".to_owned(),
                serde_json::json!({"b": true, "parallel_tool_calls": true}),
            )]),
        };

        let merged = base.merged_with(&patch);
        assert_eq!(merged.headers["x-mode"], "fast");
        assert_eq!(
            merged.body_patch["options"],
            serde_json::json!({"a": 1, "b": true})
        );
        assert!(!merged.body_patch.contains_key("parallel_tool_calls"));
    }
}
