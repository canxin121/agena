use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn prompt_cache_header_entries(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut entries = headers
        .iter()
        .filter_map(|(key, value)| {
            let normalized_key = key.trim().to_ascii_lowercase();
            let normalized_value = value.trim();
            if normalized_key.is_empty()
                || normalized_value.is_empty()
                || prompt_cache_ignores_header(normalized_key.as_str())
            {
                return None;
            }
            Some((normalized_key, normalized_value.to_owned()))
        })
        .collect::<Vec<_>>();
    entries.sort_unstable();
    entries
}

pub fn prompt_cache_ignores_header(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    key == "authorization"
        || key == "proxy-authorization"
        || key == "cookie"
        || key == "set-cookie"
        || key == "x-api-key"
        || key.contains("request-id")
        || key.contains("correlation-id")
        || key.contains("trace")
        || key.contains("span")
        || key.contains("baggage")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("signature")
}

pub fn request_shape_fingerprint<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_slice());
    hex::encode(hasher.finalize())
}

pub fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

pub fn auth_header_value(scheme: Option<&str>, token: &str) -> String {
    let token = token.trim();
    match scheme.map(str::trim).filter(|scheme| !scheme.is_empty()) {
        Some(scheme) => format!("{scheme} {token}"),
        None => token.to_owned(),
    }
}

pub fn insert_header_case_insensitive(
    headers: &mut HashMap<String, String>,
    key: impl Into<String>,
    value: impl Into<String>,
) {
    let key = key.into();
    headers.retain(|existing, _| !existing.eq_ignore_ascii_case(key.as_str()));
    headers.insert(key, value.into());
}

pub fn ensure_header_case_insensitive<F>(headers: &mut HashMap<String, String>, key: &str, value: F)
where
    F: FnOnce() -> String,
{
    if headers
        .keys()
        .any(|existing| existing.eq_ignore_ascii_case(key))
    {
        return;
    }
    headers.insert(key.to_owned(), value());
}

pub fn merged_request_headers(
    base_headers: &HashMap<String, String>,
    request_headers: &BTreeMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = base_headers.clone();
    for (key, value) in request_headers {
        merged.retain(|existing, _| !existing.eq_ignore_ascii_case(key));
        merged.insert(key.clone(), value.clone());
    }
    merged
}

pub fn merge_json_object_patch_map(
    target: &mut serde_json::Map<String, serde_json::Value>,
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

/// Merge provider replay metadata emitted across several stream events.
///
/// Terminal events are snapshots, but compatible gateways occasionally emit
/// more than one of them (or split replay fields across them). Treat absent or
/// structurally empty updates as "no change" so a late `{}`, `null`, or blank
/// value cannot erase an earlier response id or reasoning payload. Object
/// updates merge recursively; meaningful later scalar/array values replace
/// their earlier counterpart.
pub fn merge_provider_metadata(
    current: Option<serde_json::Value>,
    update: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut current = current.filter(provider_metadata_value_is_meaningful);
    let Some(update) = update.filter(provider_metadata_value_is_meaningful) else {
        return current;
    };
    match current.as_mut() {
        Some(current) => merge_provider_metadata_value(current, update),
        None => current = Some(update),
    }
    current
}

fn merge_provider_metadata_value(current: &mut serde_json::Value, update: serde_json::Value) {
    if !provider_metadata_value_is_meaningful(&update) {
        return;
    }
    match (current, update) {
        (serde_json::Value::Object(current), serde_json::Value::Object(update)) => {
            for (key, value) in update {
                if !provider_metadata_value_is_meaningful(&value) {
                    continue;
                }
                match current.get_mut(key.as_str()) {
                    Some(existing) => merge_provider_metadata_value(existing, value),
                    None => {
                        current.insert(key, value);
                    }
                }
            }
        }
        (current, update) => *current = update,
    }
}

/// Whether a provider replay-metadata value carries any non-empty state.
///
/// Objects and arrays are inspected recursively so wrappers containing only
/// `null`, blank strings, or empty containers behave like an absent update.
pub fn provider_metadata_value_is_meaningful(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(value) => value.iter().any(provider_metadata_value_is_meaningful),
        serde_json::Value::Object(value) => {
            value.values().any(provider_metadata_value_is_meaningful)
        }
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
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

pub fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

pub fn optional_non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_header_entries_are_normalized_sorted_and_secret_safe() {
        let headers = HashMap::from([
            ("X-Trace-Id".to_owned(), "discard".to_owned()),
            ("X-Feature".to_owned(), " enabled ".to_owned()),
            ("Authorization".to_owned(), "secret".to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ]);

        assert_eq!(
            prompt_cache_header_entries(&headers),
            vec![
                ("accept".to_owned(), "application/json".to_owned()),
                ("x-feature".to_owned(), "enabled".to_owned()),
            ]
        );
    }

    #[test]
    fn provider_metadata_merge_preserves_earlier_replay_state_from_empty_updates() {
        let current = serde_json::json!({
            "response_id": "resp_1",
            "nested": {
                "reasoning": [1, 2],
                "first": true
            }
        });
        let update = serde_json::json!({
            "response_id": "   ",
            "nested": {
                "reasoning": [],
                "first": null,
                "second": true
            }
        });

        let merged = merge_provider_metadata(Some(current), Some(update))
            .expect("meaningful replay metadata remains");
        assert_eq!(merged["response_id"], "resp_1");
        assert_eq!(merged["nested"]["reasoning"], serde_json::json!([1, 2]));
        assert_eq!(merged["nested"]["first"], true);
        assert_eq!(merged["nested"]["second"], true);
        assert_eq!(
            merge_provider_metadata(Some(merged.clone()), Some(serde_json::json!({}))),
            Some(merged),
            "an empty terminal metadata snapshot is not a deletion"
        );
        assert_eq!(
            merge_provider_metadata(
                None,
                Some(serde_json::json!({
                    "response_id": "   ",
                    "nested": { "reasoning": [], "value": null }
                })),
            ),
            None,
            "a recursively empty first snapshot is not replay state"
        );
    }

    #[test]
    fn json_object_patch_merges_nested_objects_without_losing_siblings() {
        let mut target = serde_json::json!({
            "body": { "keep": true, "replace": "old" },
            "unchanged": 1,
        })
        .as_object()
        .expect("object")
        .clone();
        let patch = BTreeMap::from([(
            "body".to_owned(),
            serde_json::json!({ "replace": "new", "added": 2 }),
        )]);

        merge_json_object_patch_map(&mut target, &patch);

        assert_eq!(
            serde_json::Value::Object(target),
            serde_json::json!({
                "body": { "keep": true, "replace": "new", "added": 2 },
                "unchanged": 1,
            })
        );
    }

    #[test]
    fn optional_text_variants_preserve_their_existing_whitespace_contracts() {
        assert_eq!(
            normalize_optional_text(Some("  value  ".to_owned())),
            Some("value".to_owned())
        );
        assert_eq!(normalize_optional_text(Some("  ".to_owned())), None);
        assert_eq!(
            optional_non_empty(Some("  ".to_owned())),
            Some("  ".to_owned())
        );
        assert_eq!(optional_non_empty(Some(String::new())), None);
    }
}
