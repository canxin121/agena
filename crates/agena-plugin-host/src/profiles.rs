//! Stable-id plugin profile composition inspired by DeepSeek Harness bundles.
//!
//! Profiles are deployment-time layers over the single `plugins.list` leaf.
//! Parents are applied before children, active profiles are expanded in the
//! declared order, and every mutation records a deterministic trace. Runtime
//! consumers never interpret profile syntax: they receive only the resolved
//! list plus non-wire provenance metadata.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{ConfiguredPlugin, PluginActivationConfig, PluginPackage, TimeoutsConfig};
use crate::sdk::PluginKey;

const MAX_PROFILE_ID_BYTES: usize = 128;
const MAX_PROFILE_COUNT: usize = 128;
const MAX_PROFILE_EXTENDS: usize = 32;
const MAX_PROFILE_ENTRIES: usize = 512;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginProfile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, PluginProfileEntry>,
}

fn profile_change_paths(before: Option<&Value>, after: Option<&Value>) -> Vec<String> {
    let (Some(before), Some(after)) = (before, after) else {
        return vec!["$".to_string()];
    };
    let mut paths = Vec::new();
    collect_changed_paths(before, after, String::new(), &mut paths);
    if paths.is_empty() {
        return Vec::new();
    }
    paths.sort();
    paths.dedup();
    paths
}

fn collect_changed_paths(before: &Value, after: &Value, path: String, output: &mut Vec<String>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            let keys = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
            for key in keys {
                let child_path = format!("{}/{}", path, escape_pointer_segment(key));
                match (before.get(key), after.get(key)) {
                    (Some(left), Some(right)) => {
                        collect_changed_paths(left, right, child_path, output)
                    }
                    _ => output.push(if child_path.is_empty() {
                        "$".to_string()
                    } else {
                        child_path
                    }),
                }
            }
        }
        _ => output.push(if path.is_empty() {
            "$".to_string()
        } else {
            path
        }),
    }
}

fn escape_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginProfileEntry {
    /// Add a new stable-id row or replace an existing row completely.
    Replace {
        plugin: ConfiguredPlugin,
    },
    /// Patch an existing row. `settings_patch` uses JSON Merge Patch semantics:
    /// object keys merge recursively and nested null values delete keys.
    Patch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<PluginPackage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settings_patch: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeouts: Option<TimeoutsConfig>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activation: Option<PluginActivationConfig>,
    },
    Disable,
    Remove,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginProfileAction {
    Replace,
    Patch,
    Disable,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginProfileChange {
    pub profile: String,
    pub plugin_id: String,
    pub action: PluginProfileAction,
    /// Stable JSON-pointer-like paths changed by this profile step. `$`
    /// denotes whole-row insertion/removal/replacement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginProfileResolutionMeta {
    /// Runtime-only sentinel preventing a resolved config from being applied a
    /// second time when it crosses the Runtime/Host composition boundary.
    #[serde(default, skip_serializing)]
    pub resolved: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<PluginProfileChange>,
}

impl PluginProfileResolutionMeta {
    pub fn is_empty(&self) -> bool {
        self.applied_profiles.is_empty() && self.changes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginProfileResolution {
    pub list: BTreeMap<String, ConfiguredPlugin>,
    pub meta: PluginProfileResolutionMeta,
}

pub fn resolve_plugin_profiles(
    base: &BTreeMap<String, ConfiguredPlugin>,
    profiles: &BTreeMap<String, PluginProfile>,
    active_profiles: &[String],
) -> Result<PluginProfileResolution, String> {
    validate_profiles(profiles, active_profiles)?;
    let order = expand_profile_order(profiles, active_profiles)?;
    let mut list = base.clone();
    let mut changes = Vec::new();

    for profile_id in &order {
        let profile = profiles
            .get(profile_id)
            .expect("expanded profile order references a validated profile");
        for (plugin_id, entry) in &profile.plugins {
            plugin_id.parse::<PluginKey>().map_err(|error| {
                format!("profile `{profile_id}` contains invalid plugin id `{plugin_id}`: {error}")
            })?;
            let before = list
                .get(plugin_id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| {
                    format!("serialize plugin `{plugin_id}` before profile `{profile_id}`: {error}")
                })?;
            let action = apply_entry(&mut list, profile_id, plugin_id, entry)?;
            let after = list
                .get(plugin_id)
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| {
                    format!("serialize plugin `{plugin_id}` after profile `{profile_id}`: {error}")
                })?;
            changes.push(PluginProfileChange {
                profile: profile_id.clone(),
                plugin_id: plugin_id.clone(),
                action,
                paths: profile_change_paths(before.as_ref(), after.as_ref()),
            });
        }
    }

    Ok(PluginProfileResolution {
        list,
        meta: PluginProfileResolutionMeta {
            resolved: true,
            applied_profiles: order,
            changes,
        },
    })
}

pub fn validate_profiles(
    profiles: &BTreeMap<String, PluginProfile>,
    active_profiles: &[String],
) -> Result<(), String> {
    if profiles.len() > MAX_PROFILE_COUNT {
        return Err(format!(
            "plugin profile count exceeds the {MAX_PROFILE_COUNT} profile limit"
        ));
    }
    let mut active = BTreeSet::new();
    for profile_id in active_profiles {
        validate_profile_id(profile_id)?;
        if !active.insert(profile_id) {
            return Err(format!(
                "active plugin profile `{profile_id}` is listed more than once"
            ));
        }
        if !profiles.contains_key(profile_id) {
            return Err(format!(
                "active plugin profile `{profile_id}` is not declared under plugins.profiles"
            ));
        }
    }
    for (profile_id, profile) in profiles {
        validate_profile_id(profile_id)?;
        if profile.extends.len() > MAX_PROFILE_EXTENDS {
            return Err(format!(
                "plugin profile `{profile_id}` extends more than {MAX_PROFILE_EXTENDS} profiles"
            ));
        }
        if profile.plugins.len() > MAX_PROFILE_ENTRIES {
            return Err(format!(
                "plugin profile `{profile_id}` contains more than {MAX_PROFILE_ENTRIES} plugin entries"
            ));
        }
        let mut parents = BTreeSet::new();
        for parent in &profile.extends {
            validate_profile_id(parent)?;
            if parent == profile_id {
                return Err(format!(
                    "plugin profile `{profile_id}` cannot extend itself"
                ));
            }
            if !parents.insert(parent) {
                return Err(format!(
                    "plugin profile `{profile_id}` extends `{parent}` more than once"
                ));
            }
            if !profiles.contains_key(parent) {
                return Err(format!(
                    "plugin profile `{profile_id}` extends unknown profile `{parent}`"
                ));
            }
        }
    }
    // Expansion performs cycle detection even when no cyclic profile is active,
    // so a dormant typo cannot become a deployment-time surprise later.
    for profile_id in profiles.keys() {
        let _ = expand_profile_order(profiles, std::slice::from_ref(profile_id))?;
    }
    Ok(())
}

fn expand_profile_order(
    profiles: &BTreeMap<String, PluginProfile>,
    active_profiles: &[String],
) -> Result<Vec<String>, String> {
    fn visit(
        profile_id: &str,
        profiles: &BTreeMap<String, PluginProfile>,
        visiting: &mut Vec<String>,
        applied: &mut BTreeSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if applied.contains(profile_id) {
            return Ok(());
        }
        if let Some(index) = visiting
            .iter()
            .position(|candidate| candidate == profile_id)
        {
            let mut cycle = visiting[index..].to_vec();
            cycle.push(profile_id.to_string());
            return Err(format!(
                "plugin profile inheritance cycle: {}",
                cycle.join(" -> ")
            ));
        }
        let profile = profiles.get(profile_id).ok_or_else(|| {
            format!("plugin profile `{profile_id}` is not declared under plugins.profiles")
        })?;
        visiting.push(profile_id.to_string());
        for parent in &profile.extends {
            visit(parent, profiles, visiting, applied, order)?;
        }
        visiting.pop();
        if applied.insert(profile_id.to_string()) {
            order.push(profile_id.to_string());
        }
        Ok(())
    }

    let mut visiting = Vec::new();
    let mut applied = BTreeSet::new();
    let mut order = Vec::new();
    for profile_id in active_profiles {
        visit(
            profile_id,
            profiles,
            &mut visiting,
            &mut applied,
            &mut order,
        )?;
    }
    Ok(order)
}

fn apply_entry(
    list: &mut BTreeMap<String, ConfiguredPlugin>,
    profile_id: &str,
    plugin_id: &str,
    entry: &PluginProfileEntry,
) -> Result<PluginProfileAction, String> {
    match entry {
        PluginProfileEntry::Replace { plugin } => {
            list.insert(plugin_id.to_string(), plugin.clone());
            Ok(PluginProfileAction::Replace)
        }
        PluginProfileEntry::Patch {
            enabled,
            package,
            settings_patch,
            timeouts,
            activation,
        } => {
            let configured = list.get_mut(plugin_id).ok_or_else(|| {
                format!(
                    "plugin profile `{profile_id}` cannot patch missing plugin `{plugin_id}`; use action=replace to add it"
                )
            })?;
            if let Some(enabled) = enabled {
                configured.enabled = *enabled;
            }
            if let Some(package) = package {
                configured.package = package.clone();
            }
            if let Some(settings_patch) = settings_patch {
                apply_json_merge_patch(&mut configured.settings, settings_patch);
            }
            if let Some(timeouts) = timeouts {
                configured.timeouts = timeouts.clone();
            }
            if let Some(activation) = activation {
                configured.activation = activation.clone();
            }
            Ok(PluginProfileAction::Patch)
        }
        PluginProfileEntry::Disable => {
            let configured = list.get_mut(plugin_id).ok_or_else(|| {
                format!("plugin profile `{profile_id}` cannot disable missing plugin `{plugin_id}`")
            })?;
            configured.enabled = false;
            Ok(PluginProfileAction::Disable)
        }
        PluginProfileEntry::Remove => {
            if list.remove(plugin_id).is_none() {
                return Err(format!(
                    "plugin profile `{profile_id}` cannot remove missing plugin `{plugin_id}`"
                ));
            }
            Ok(PluginProfileAction::Remove)
        }
    }
}

/// RFC 7396 JSON Merge Patch.
pub fn apply_json_merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch_object) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Default::default());
    }
    let target_object = target
        .as_object_mut()
        .expect("target was normalized to an object");
    for (key, value) in patch_object {
        if value.is_null() {
            target_object.remove(key);
            continue;
        }
        apply_json_merge_patch(
            target_object.entry(key.clone()).or_insert(Value::Null),
            value,
        );
    }
}

fn validate_profile_id(profile_id: &str) -> Result<(), String> {
    if profile_id.is_empty() {
        return Err("plugin profile id must not be empty".to_string());
    }
    if profile_id.len() > MAX_PROFILE_ID_BYTES {
        return Err(format!(
            "plugin profile id exceeds {MAX_PROFILE_ID_BYTES} bytes"
        ));
    }
    if profile_id.starts_with('.')
        || profile_id.ends_with('.')
        || profile_id.chars().any(|character| {
            !character.is_ascii_alphanumeric() && !matches!(character, '.' | '_' | '-')
        })
    {
        return Err(format!(
            "invalid plugin profile id `{profile_id}`; use ASCII letters, digits, '.', '_' or '-'"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(settings: Value) -> ConfiguredPlugin {
        ConfiguredPlugin::static_settings(settings)
    }

    fn patch(settings_patch: Value) -> PluginProfileEntry {
        PluginProfileEntry::Patch {
            enabled: None,
            package: None,
            settings_patch: Some(settings_patch),
            timeouts: None,
            activation: None,
        }
    }

    #[test]
    fn parents_apply_once_before_children_and_changes_are_traced() {
        let base = BTreeMap::from([(
            "example.plugin".to_string(),
            configured(serde_json::json!({
                "mode": "base",
                "nested": {"keep": true, "remove": true}
            })),
        )]);
        let profiles = BTreeMap::from([
            (
                "base-tools".to_string(),
                PluginProfile {
                    plugins: BTreeMap::from([(
                        "example.plugin".to_string(),
                        patch(serde_json::json!({
                            "mode": "profile-base",
                            "nested": {"remove": null, "base": 1}
                        })),
                    )]),
                    ..PluginProfile::default()
                },
            ),
            (
                "coding".to_string(),
                PluginProfile {
                    extends: vec!["base-tools".to_string()],
                    plugins: BTreeMap::from([(
                        "example.plugin".to_string(),
                        patch(serde_json::json!({"nested": {"child": 2}})),
                    )]),
                },
            ),
        ]);

        let resolution = resolve_plugin_profiles(
            &base,
            &profiles,
            &["coding".to_string(), "base-tools".to_string()],
        )
        .expect("resolve profiles");

        assert_eq!(resolution.meta.applied_profiles, ["base-tools", "coding"]);
        assert_eq!(
            resolution.list["example.plugin"].settings(),
            &serde_json::json!({
                "mode": "profile-base",
                "nested": {"keep": true, "base": 1, "child": 2}
            })
        );
        assert_eq!(resolution.meta.changes.len(), 2);
        assert_eq!(
            resolution.meta.changes[0].paths,
            [
                "/settings/mode",
                "/settings/nested/base",
                "/settings/nested/remove"
            ]
        );
        assert_eq!(resolution.meta.changes[1].paths, ["/settings/nested/child"]);
    }

    #[test]
    fn replace_disable_and_remove_are_explicit_stable_id_operations() {
        let base = BTreeMap::from([
            ("example.disable".to_string(), configured(Value::Null)),
            ("example.remove".to_string(), configured(Value::Null)),
        ]);
        let profiles = BTreeMap::from([(
            "minimal".to_string(),
            PluginProfile {
                plugins: BTreeMap::from([
                    ("example.disable".to_string(), PluginProfileEntry::Disable),
                    ("example.remove".to_string(), PluginProfileEntry::Remove),
                    (
                        "example.add".to_string(),
                        PluginProfileEntry::Replace {
                            plugin: configured(serde_json::json!({"added": true})),
                        },
                    ),
                ]),
                ..PluginProfile::default()
            },
        )]);

        let resolution = resolve_plugin_profiles(&base, &profiles, &["minimal".to_string()])
            .expect("resolve profile");

        assert!(resolution.list["example.disable"].disabled());
        assert!(!resolution.list.contains_key("example.remove"));
        assert_eq!(
            resolution.list["example.add"].settings(),
            &serde_json::json!({"added": true})
        );
        let changes = resolution
            .meta
            .changes
            .iter()
            .map(|change| (change.plugin_id.as_str(), change.paths.as_slice()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(changes["example.remove"], ["$"]);
        assert_eq!(changes["example.add"], ["$"]);
    }

    #[test]
    fn profile_graph_and_missing_target_errors_are_strict() {
        let cycle = BTreeMap::from([
            (
                "a".to_string(),
                PluginProfile {
                    extends: vec!["b".to_string()],
                    ..PluginProfile::default()
                },
            ),
            (
                "b".to_string(),
                PluginProfile {
                    extends: vec!["a".to_string()],
                    ..PluginProfile::default()
                },
            ),
        ]);
        assert!(
            validate_profiles(&cycle, &["a".to_string()])
                .expect_err("cycle")
                .contains("cycle")
        );

        let missing = BTreeMap::from([(
            "bad".to_string(),
            PluginProfile {
                plugins: BTreeMap::from([(
                    "example.missing".to_string(),
                    patch(serde_json::json!({"value": 1})),
                )]),
                ..PluginProfile::default()
            },
        )]);
        assert!(
            resolve_plugin_profiles(&BTreeMap::new(), &missing, &["bad".to_string()])
                .expect_err("missing patch target")
                .contains("action=replace")
        );
    }

    #[test]
    fn merge_patch_can_replace_scalars_and_delete_nested_keys() {
        let mut value = serde_json::json!({"a": 1, "nested": {"x": 1, "y": 2}});
        apply_json_merge_patch(
            &mut value,
            &serde_json::json!({"a": [1, 2], "nested": {"x": null, "z": 3}}),
        );
        assert_eq!(
            value,
            serde_json::json!({"a": [1, 2], "nested": {"y": 2, "z": 3}})
        );
    }
}
