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

    pub fn parallel_tool_calls(&self) -> Option<bool> {
        self.body_patch
            .get("parallel_tool_calls")
            .and_then(serde_json::Value::as_bool)
    }

    pub fn set_parallel_tool_calls(&mut self, enabled: Option<bool>) {
        match enabled {
            Some(enabled) => {
                self.body_patch.insert(
                    "parallel_tool_calls".to_owned(),
                    serde_json::Value::Bool(enabled),
                );
            }
            None => {
                self.body_patch.remove("parallel_tool_calls");
            }
        }
    }

    pub fn merged_with(&self, other: &Self) -> Self {
        let mut merged = self.clone();
        for (key, value) in &other.headers {
            merged.headers.insert(key.clone(), value.clone());
        }
        merge_json_patch_maps(&mut merged.body_patch, &other.body_patch);
        merged
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
            body_patch: BTreeMap::from([
                ("options".to_owned(), serde_json::json!({"b": true})),
                ("parallel_tool_calls".to_owned(), serde_json::json!(true)),
            ]),
        };

        let merged = base.merged_with(&patch);
        assert_eq!(merged.headers["x-mode"], "fast");
        assert_eq!(
            merged.body_patch["options"],
            serde_json::json!({"a": 1, "b": true})
        );
        assert_eq!(merged.parallel_tool_calls(), Some(true));
    }
}
