use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

static SERIALIZATION_FAILURE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn cache_shape_json<T: Serialize>(value: &T, operation: &str) -> String {
    match serde_json::to_string(value) {
        Ok(encoded) => encoded,
        Err(error) => {
            let sequence = SERIALIZATION_FAILURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                operation,
                sequence,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    operation,
                    &error,
                ),
                "prompt-cache shape value could not be serialized; disabling cache-key reuse for this value"
            );
            format!("__agena_prompt_cache_serialization_failure_{sequence}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Shape of a prompt for cache key computation.
pub struct PromptCacheShape {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

impl PromptCacheShape {
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            fields: BTreeMap::new(),
        }
    }

    pub fn from_fields<K, V>(
        provider_id: impl Into<String>,
        fields: impl IntoIterator<Item = (K, V)>,
    ) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            provider_id: provider_id.into(),
            fields: fields
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    pub fn provider_id(&self) -> &str {
        self.provider_id.as_str()
    }

    pub fn json_field_value<T>(value: &T) -> String
    where
        T: Serialize,
    {
        cache_shape_json(value, "serialize prompt-cache shape field")
    }

    pub fn insert_string(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.fields.insert(key.into(), value.into());
    }

    pub fn insert_bool(&mut self, key: impl Into<String>, value: bool) {
        self.fields.insert(key.into(), value.to_string());
    }

    pub fn insert_json<T>(&mut self, key: impl Into<String>, value: &T)
    where
        T: Serialize,
    {
        let encoded = cache_shape_json(value, "serialize inserted prompt-cache shape field");
        self.fields.insert(key.into(), encoded);
    }

    pub fn extend_prefixed(&mut self, prefix: &str, shape: &PromptCacheShape) {
        let prefix = prefix.trim().trim_matches('.');
        if prefix.is_empty() {
            return;
        }

        self.insert_string(
            format!("{prefix}.provider_id"),
            shape.provider_id.as_str().to_owned(),
        );
        for (key, value) in &shape.fields {
            self.insert_string(format!("{prefix}.{key}"), value.clone());
        }
    }

    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.provider_id.len().to_le_bytes());
        hasher.update(self.provider_id.as_bytes());
        hasher.update(self.fields.len().to_le_bytes());
        for (key, value) in &self.fields {
            hasher.update(key.len().to_le_bytes());
            hasher.update(key.as_bytes());
            hasher.update(value.len().to_le_bytes());
            hasher.update(value.as_bytes());
        }
        hex::encode(hasher.finalize())
    }

    pub fn diff(previous: Option<&Self>, current: Option<&Self>) -> PromptCacheShapeDiff {
        let mut keys = BTreeSet::new();
        if let Some(shape) = previous {
            keys.insert("provider_id".to_owned());
            keys.extend(shape.fields.keys().cloned());
        }
        if let Some(shape) = current {
            keys.insert("provider_id".to_owned());
            keys.extend(shape.fields.keys().cloned());
        }

        let changes = keys
            .into_iter()
            .filter_map(|key| {
                let previous_value = field_value(previous, key.as_str());
                let current_value = field_value(current, key.as_str());
                (previous_value != current_value).then_some(PromptCacheShapeChange {
                    key,
                    previous: previous_value,
                    current: current_value,
                })
            })
            .collect();

        PromptCacheShapeDiff { changes }
    }
}

fn field_value(shape: Option<&PromptCacheShape>, key: &str) -> Option<String> {
    let shape = shape?;
    if key == "provider_id" {
        return Some(shape.provider_id.clone());
    }
    shape.fields.get(key).cloned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A single change in prompt cache shape.
pub struct PromptCacheShapeChange {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Diff of prompt cache shape between revisions.
pub struct PromptCacheShapeDiff {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<PromptCacheShapeChange>,
}

impl PromptCacheShapeDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn changed_keys(&self) -> Vec<&str> {
        self.changes
            .iter()
            .map(|change| change.key.as_str())
            .collect()
    }
}
