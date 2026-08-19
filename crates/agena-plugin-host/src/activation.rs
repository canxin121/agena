//! Deterministic dependency-driven activation for configured plugin instances.
//!
//! This is intentionally transport- and SDK-neutral. The planner consumes the
//! already-resolved `plugins.list` map and produces a stable load order plus
//! explicit blockers. Runtime initialization failures are handled by the host
//! build loop with the same required-dependency rule.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::config::ConfiguredPlugin;
use crate::sdk::PluginKey;

/// A configured plugin that cannot enter its initialization phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginActivationBlock {
    pub plugin_id: String,
    pub code: &'static str,
    pub message: String,
    /// Required dependency ids directly responsible for the block.
    pub dependencies: Vec<String>,
}

/// Stable activation order and plugins rejected before transport loading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginActivationPlan {
    pub ordered: Vec<String>,
    pub blocked: BTreeMap<String, PluginActivationBlock>,
}

/// Validate dependency declarations and compute a deterministic activation
/// order. Hard `requires` edges participate in cycle detection. Soft `after`
/// hints influence selection among currently-ready nodes, but a hint can never
/// block activation or create a cycle.
pub fn plan_plugin_activation(
    plugins: &BTreeMap<String, ConfiguredPlugin>,
) -> Result<PluginActivationPlan, String> {
    let mut enabled = BTreeSet::new();
    for (plugin_id, configured) in plugins {
        plugin_id
            .parse::<PluginKey>()
            .map_err(|error| format!("invalid configured plugin id `{plugin_id}`: {error}"))?;
        validate_declaration(plugin_id, configured)?;
        if configured.enabled {
            enabled.insert(plugin_id.clone());
        }
    }

    let mut blocked = BTreeMap::<String, PluginActivationBlock>::new();
    for plugin_id in &enabled {
        let configured = &plugins[plugin_id];
        let unavailable = configured
            .activation
            .requires
            .iter()
            .filter_map(|dependency| {
                let dependency = dependency.to_string();
                match plugins.get(&dependency) {
                    None => Some((dependency, "missing")),
                    Some(entry) if !entry.enabled => Some((dependency, "disabled")),
                    Some(_) => None,
                }
            })
            .collect::<Vec<_>>();
        if unavailable.is_empty() {
            continue;
        }
        let dependencies = unavailable
            .iter()
            .map(|(dependency, _)| dependency.clone())
            .collect::<Vec<_>>();
        let detail = unavailable
            .iter()
            .map(|(dependency, state)| format!("`{dependency}` ({state})"))
            .collect::<Vec<_>>()
            .join(", ");
        blocked.insert(
            plugin_id.clone(),
            PluginActivationBlock {
                plugin_id: plugin_id.clone(),
                code: "required_dependency_unavailable",
                message: format!("required plugin dependency unavailable: {detail}"),
                dependencies,
            },
        );
    }

    // A blocked requirement blocks every transitive dependent before any
    // transport is started. Iterate to a fixed point so diagnostics remain
    // deterministic even when the declaration order changes.
    loop {
        let mut newly_blocked = Vec::new();
        for plugin_id in &enabled {
            if blocked.contains_key(plugin_id) {
                continue;
            }
            let dependencies = required_ids(&plugins[plugin_id])
                .filter(|dependency| blocked.contains_key(dependency))
                .collect::<Vec<_>>();
            if dependencies.is_empty() {
                continue;
            }
            newly_blocked.push((plugin_id.clone(), dependencies));
        }
        if newly_blocked.is_empty() {
            break;
        }
        for (plugin_id, dependencies) in newly_blocked {
            blocked.insert(
                plugin_id.clone(),
                PluginActivationBlock {
                    plugin_id,
                    code: "required_dependency_blocked",
                    message: format!(
                        "required plugin dependency is blocked: {}",
                        dependencies
                            .iter()
                            .map(|dependency| format!("`{dependency}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    dependencies,
                },
            );
        }
    }

    let candidates = enabled
        .iter()
        .filter(|plugin_id| !blocked.contains_key(*plugin_id))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut dependents = BTreeMap::<String, BTreeSet<String>>::new();
    let mut indegree = BTreeMap::<String, usize>::new();
    for plugin_id in &candidates {
        indegree.insert(plugin_id.clone(), 0);
    }
    for plugin_id in &candidates {
        for dependency in required_ids(&plugins[plugin_id]) {
            if !candidates.contains(&dependency) {
                continue;
            }
            dependents
                .entry(dependency)
                .or_default()
                .insert(plugin_id.clone());
            *indegree
                .get_mut(plugin_id)
                .expect("candidate indegree must exist") += 1;
        }
    }

    let mut ready = indegree
        .iter()
        .filter_map(|(plugin_id, count)| (*count == 0).then_some(plugin_id.clone()))
        .collect::<BTreeSet<_>>();
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::with_capacity(candidates.len());
    while !ready.is_empty() {
        // Honor `after` only when all present candidate hints have already
        // emitted. If every ready node participates in a soft cycle, lexical
        // order breaks the tie without changing activation semantics.
        let selected = ready
            .iter()
            .find(|plugin_id| {
                after_ids(&plugins[*plugin_id]).all(|dependency| {
                    !candidates.contains(&dependency) || emitted.contains(&dependency)
                })
            })
            .cloned()
            .unwrap_or_else(|| ready.first().expect("ready is non-empty").clone());
        ready.remove(&selected);
        emitted.insert(selected.clone());
        ordered.push(selected.clone());
        if let Some(entries) = dependents.get(&selected) {
            for dependent in entries {
                let count = indegree
                    .get_mut(dependent)
                    .expect("dependent indegree must exist");
                *count -= 1;
                if *count == 0 {
                    ready.insert(dependent.clone());
                }
            }
        }
    }

    let unresolved = candidates
        .difference(&emitted)
        .cloned()
        .collect::<BTreeSet<_>>();
    for plugin_id in &unresolved {
        let dependencies = required_ids(&plugins[plugin_id])
            .filter(|dependency| unresolved.contains(dependency))
            .collect::<Vec<_>>();
        blocked.insert(
            plugin_id.clone(),
            PluginActivationBlock {
                plugin_id: plugin_id.clone(),
                code: "required_dependency_cycle",
                message: format!(
                    "required plugin dependency cycle or cyclic upstream dependency: {}",
                    dependencies
                        .iter()
                        .map(|dependency| format!("`{dependency}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                dependencies,
            },
        );
    }

    Ok(PluginActivationPlan { ordered, blocked })
}

fn validate_declaration(plugin_id: &str, configured: &ConfiguredPlugin) -> Result<(), String> {
    let mut required = BTreeSet::new();
    for dependency in &configured.activation.requires {
        let dependency = dependency.to_string();
        if dependency == plugin_id {
            return Err(format!(
                "plugin `{plugin_id}` cannot require itself in activation.requires"
            ));
        }
        if !required.insert(dependency.clone()) {
            return Err(format!(
                "plugin `{plugin_id}` declares duplicate required dependency `{dependency}`"
            ));
        }
    }
    let mut after = BTreeSet::new();
    for dependency in &configured.activation.after {
        let dependency = dependency.to_string();
        if dependency == plugin_id {
            return Err(format!(
                "plugin `{plugin_id}` cannot order itself in activation.after"
            ));
        }
        if required.contains(&dependency) {
            return Err(format!(
                "plugin `{plugin_id}` declares `{dependency}` in both activation.requires and activation.after"
            ));
        }
        if !after.insert(dependency.clone()) {
            return Err(format!(
                "plugin `{plugin_id}` declares duplicate ordering dependency `{dependency}`"
            ));
        }
    }
    Ok(())
}

fn required_ids(configured: &ConfiguredPlugin) -> impl Iterator<Item = String> + '_ {
    configured
        .activation
        .requires
        .iter()
        .map(ToString::to_string)
}

fn after_ids(configured: &ConfiguredPlugin) -> impl Iterator<Item = String> + '_ {
    configured.activation.after.iter().map(ToString::to_string)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginReloadAction {
    Add,
    Reuse,
    Restart,
    Remove,
    Disabled,
    Blocked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PluginReloadReason {
    Added,
    Removed,
    Enabled,
    Disabled,
    ConfigurationChanged,
    DependencyEpochChanged,
    ServiceBindingChanged,
    BlockerChanged,
    InProcessStatic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginReloadDecision {
    pub plugin_id: PluginKey,
    pub action: PluginReloadAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<PluginReloadReason>,
    /// Direct upstream plugins whose changed activation/provider epoch forced
    /// this plugin to restart. Empty means the decision is self-caused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggered_by: Vec<PluginKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginReloadPlan {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<PluginReloadDecision>,
}

impl PluginReloadPlan {
    pub fn reusable_plugin_ids(&self) -> BTreeSet<String> {
        self.decisions
            .iter()
            .filter(|decision| decision.action == PluginReloadAction::Reuse)
            .map(|decision| decision.plugin_id.to_string())
            .collect()
    }

    pub fn decision(&self, plugin_id: &str) -> Option<&PluginReloadDecision> {
        self.decisions
            .iter()
            .find(|decision| decision.plugin_id.to_string() == plugin_id)
    }
}

/// Diff two resolved configured plugin trees into explicit loader decisions.
/// The comparison uses recursive activation epochs, so a transitive provider
/// change restarts every affected hard consumer while unrelated external
/// transports remain reusable.
pub fn plan_plugin_reload(
    previous: &BTreeMap<String, ConfiguredPlugin>,
    current: &BTreeMap<String, ConfiguredPlugin>,
) -> Result<PluginReloadPlan, String> {
    let previous_plan = plan_plugin_activation(previous)?;
    let current_plan = plan_plugin_activation(current)?;
    let previous_epochs = plugin_activation_epochs(previous, &previous_plan)?;
    let current_epochs = plugin_activation_epochs(current, &current_plan)?;
    let ids = previous
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut decisions = Vec::with_capacity(ids.len());

    for plugin_id in ids {
        let plugin_key = plugin_id
            .parse::<PluginKey>()
            .map_err(|error| format!("invalid configured plugin id `{plugin_id}`: {error}"))?;
        let before = previous.get(&plugin_id);
        let after = current.get(&plugin_id);
        let decision = match (before, after) {
            (Some(_), None) => PluginReloadDecision {
                plugin_id: plugin_key,
                action: PluginReloadAction::Remove,
                reasons: vec![PluginReloadReason::Removed],
                triggered_by: Vec::new(),
            },
            (None, Some(configured)) if !configured.enabled => PluginReloadDecision {
                plugin_id: plugin_key,
                action: PluginReloadAction::Disabled,
                reasons: vec![PluginReloadReason::Added, PluginReloadReason::Disabled],
                triggered_by: Vec::new(),
            },
            (None, Some(_)) if current_plan.blocked.contains_key(&plugin_id) => {
                PluginReloadDecision {
                    plugin_id: plugin_key,
                    action: PluginReloadAction::Blocked,
                    reasons: vec![
                        PluginReloadReason::Added,
                        PluginReloadReason::BlockerChanged,
                    ],
                    triggered_by: Vec::new(),
                }
            }
            (None, Some(_)) => PluginReloadDecision {
                plugin_id: plugin_key,
                action: PluginReloadAction::Add,
                reasons: vec![PluginReloadReason::Added],
                triggered_by: Vec::new(),
            },
            (Some(_), Some(configured)) if !configured.enabled => PluginReloadDecision {
                plugin_id: plugin_key,
                action: PluginReloadAction::Disabled,
                reasons: vec![PluginReloadReason::Disabled],
                triggered_by: Vec::new(),
            },
            (Some(_), Some(_)) if current_plan.blocked.contains_key(&plugin_id) => {
                let mut reasons = Vec::new();
                if previous_plan.blocked.get(&plugin_id) != current_plan.blocked.get(&plugin_id) {
                    reasons.push(PluginReloadReason::BlockerChanged);
                }
                if before != after {
                    reasons.push(PluginReloadReason::ConfigurationChanged);
                }
                PluginReloadDecision {
                    plugin_id: plugin_key,
                    action: PluginReloadAction::Blocked,
                    reasons,
                    triggered_by: Vec::new(),
                }
            }
            (Some(previous_config), Some(current_config)) => {
                let mut reasons = BTreeSet::new();
                if !previous_config.enabled && current_config.enabled {
                    reasons.insert(PluginReloadReason::Enabled);
                }
                if previous_config != current_config {
                    reasons.insert(PluginReloadReason::ConfigurationChanged);
                }
                if previous_epochs.get(&plugin_id) != current_epochs.get(&plugin_id) {
                    reasons.insert(PluginReloadReason::DependencyEpochChanged);
                }
                let mut triggered_by = current_config
                    .activation
                    .requires
                    .iter()
                    .filter(|dependency| {
                        let dependency = dependency.to_string();
                        previous_epochs.get(&dependency) != current_epochs.get(&dependency)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                triggered_by.sort();
                if matches!(
                    current_config.package,
                    crate::config::PluginPackage::Static { .. }
                ) {
                    reasons.insert(PluginReloadReason::InProcessStatic);
                }
                if previous_plan.blocked.contains_key(&plugin_id) {
                    reasons.insert(PluginReloadReason::BlockerChanged);
                }
                PluginReloadDecision {
                    plugin_id: plugin_key,
                    action: if reasons.is_empty() {
                        PluginReloadAction::Reuse
                    } else {
                        PluginReloadAction::Restart
                    },
                    reasons: reasons.into_iter().collect(),
                    triggered_by,
                }
            }
            (None, None) => unreachable!("union id must exist in one tree"),
        };
        decisions.push(decision);
    }
    decisions.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(PluginReloadPlan { decisions })
}

/// Compute one deterministic recursive activation epoch per configured plugin.
///
/// The epoch includes the plugin's own canonical configuration plus every hard
/// dependency epoch. A provider's provider therefore invalidates all affected
/// transitive consumers, while optional service providers remain late-bound and
/// do not force a consumer restart merely because their implementation changes.
pub fn plugin_activation_epochs(
    plugins: &BTreeMap<String, ConfiguredPlugin>,
    plan: &PluginActivationPlan,
) -> Result<BTreeMap<String, u64>, String> {
    let mut epochs = BTreeMap::<String, u64>::new();

    for plugin_id in &plan.ordered {
        let configured = plugins
            .get(plugin_id)
            .ok_or_else(|| format!("activation plan references unknown plugin `{plugin_id}`"))?;
        let mut hash = hash_config(plugin_id, configured)?;
        for dependency in configured
            .activation
            .requires
            .iter()
            .map(ToString::to_string)
        {
            hash_bytes(&mut hash, dependency.as_bytes());
            let dependency_epoch = epochs.get(&dependency).ok_or_else(|| {
                format!(
                    "activation epoch for `{plugin_id}` was computed before required dependency `{dependency}`"
                )
            })?;
            hash_bytes(&mut hash, &dependency_epoch.to_le_bytes());
        }
        epochs.insert(plugin_id.clone(), hash);
    }

    // Blocked and disabled rows also receive stable epochs so inspect/diff
    // surfaces can explain when their declaration changed without requiring a
    // successful activation.
    for (plugin_id, configured) in plugins {
        if epochs.contains_key(plugin_id) {
            continue;
        }
        let mut hash = hash_config(plugin_id, configured)?;
        if let Some(block) = plan.blocked.get(plugin_id) {
            hash_bytes(&mut hash, block.code.as_bytes());
            hash_bytes(&mut hash, block.message.as_bytes());
            for dependency in &block.dependencies {
                hash_bytes(&mut hash, dependency.as_bytes());
                if let Some(epoch) = epochs.get(dependency) {
                    hash_bytes(&mut hash, &epoch.to_le_bytes());
                }
            }
        }
        epochs.insert(plugin_id.clone(), hash);
    }
    Ok(epochs)
}

fn hash_config(plugin_id: &str, configured: &ConfiguredPlugin) -> Result<u64, String> {
    let mut hash = 0xcbf29ce484222325_u64;
    hash_bytes(&mut hash, plugin_id.as_bytes());
    let value = serde_json::to_value(configured).map_err(|error| {
        format!("failed to encode plugin `{plugin_id}` activation epoch: {error}")
    })?;
    hash_json(&mut hash, &value);
    Ok(hash)
}

fn hash_json(hash: &mut u64, value: &serde_json::Value) {
    match value {
        serde_json::Value::Null => hash_bytes(hash, b"n"),
        serde_json::Value::Bool(value) => {
            hash_bytes(hash, b"b");
            hash_bytes(hash, &[*value as u8]);
        }
        serde_json::Value::Number(value) => {
            hash_bytes(hash, b"d");
            hash_bytes(hash, value.to_string().as_bytes());
        }
        serde_json::Value::String(value) => {
            hash_bytes(hash, b"s");
            hash_bytes(hash, value.as_bytes());
        }
        serde_json::Value::Array(values) => {
            hash_bytes(hash, b"[");
            for value in values {
                hash_json(hash, value);
                hash_bytes(hash, b",");
            }
            hash_bytes(hash, b"]");
        }
        serde_json::Value::Object(values) => {
            hash_bytes(hash, b"{");
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                hash_bytes(hash, key.as_bytes());
                hash_bytes(hash, b":");
                hash_json(hash, &values[key]);
                hash_bytes(hash, b",");
            }
            hash_bytes(hash, b"}");
        }
    }
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PluginActivationConfig, PluginPackage};

    fn plugin(requires: &[&str], after: &[&str]) -> ConfiguredPlugin {
        ConfiguredPlugin {
            activation: PluginActivationConfig {
                requires: requires
                    .iter()
                    .map(|value| value.parse().expect("valid plugin key"))
                    .collect(),
                after: after
                    .iter()
                    .map(|value| value.parse().expect("valid plugin key"))
                    .collect(),
            },
            package: PluginPackage::Static {},
            ..ConfiguredPlugin::default()
        }
    }

    #[test]
    fn required_dependencies_drive_stable_activation_order() {
        let plugins = BTreeMap::from([
            (
                "example.consumer".to_owned(),
                plugin(&["example.provider"], &[]),
            ),
            ("example.provider".to_owned(), plugin(&[], &[])),
            ("example.unrelated".to_owned(), plugin(&[], &[])),
        ]);

        let plan = plan_plugin_activation(&plugins).expect("valid plan");

        let provider = plan
            .ordered
            .iter()
            .position(|id| id == "example.provider")
            .expect("provider is ordered");
        let consumer = plan
            .ordered
            .iter()
            .position(|id| id == "example.consumer")
            .expect("consumer is ordered");
        assert!(provider < consumer);
        assert!(plan.blocked.is_empty());
    }

    #[test]
    fn missing_disabled_and_transitively_blocked_dependencies_are_reported() {
        let mut disabled = plugin(&[], &[]);
        disabled.enabled = false;
        let plugins = BTreeMap::from([
            ("example.disabled".to_owned(), disabled),
            (
                "example.missing-consumer".to_owned(),
                plugin(&["example.missing"], &[]),
            ),
            (
                "example.disabled-consumer".to_owned(),
                plugin(&["example.disabled"], &[]),
            ),
            (
                "example.transitive".to_owned(),
                plugin(&["example.missing-consumer"], &[]),
            ),
        ]);

        let plan = plan_plugin_activation(&plugins).expect("valid plan");

        assert_eq!(
            plan.blocked["example.missing-consumer"].code,
            "required_dependency_unavailable"
        );
        assert_eq!(
            plan.blocked["example.disabled-consumer"].code,
            "required_dependency_unavailable"
        );
        assert_eq!(
            plan.blocked["example.transitive"].code,
            "required_dependency_blocked"
        );
    }

    #[test]
    fn hard_cycles_block_while_soft_cycles_fall_back_to_lexical_order() {
        let hard = BTreeMap::from([
            ("example.a".to_owned(), plugin(&["example.b"], &[])),
            ("example.b".to_owned(), plugin(&["example.a"], &[])),
        ]);
        let hard_plan = plan_plugin_activation(&hard).expect("valid declarations");
        assert!(hard_plan.ordered.is_empty());
        assert_eq!(hard_plan.blocked.len(), 2);
        assert!(
            hard_plan
                .blocked
                .values()
                .all(|block| block.code == "required_dependency_cycle")
        );

        let soft = BTreeMap::from([
            ("example.a".to_owned(), plugin(&[], &["example.b"])),
            ("example.b".to_owned(), plugin(&[], &["example.a"])),
        ]);
        let soft_plan = plan_plugin_activation(&soft).expect("valid declarations");
        assert_eq!(soft_plan.ordered, ["example.a", "example.b"]);
        assert!(soft_plan.blocked.is_empty());
    }

    #[test]
    fn soft_after_hint_orders_ready_plugins_without_becoming_a_requirement() {
        let plugins = BTreeMap::from([
            ("example.a".to_owned(), plugin(&[], &["example.z"])),
            ("example.z".to_owned(), plugin(&[], &[])),
        ]);
        let plan = plan_plugin_activation(&plugins).expect("valid plan");
        assert_eq!(plan.ordered, ["example.z", "example.a"]);
    }

    #[test]
    fn activation_epoch_changes_propagate_through_the_hard_dependency_closure() {
        let mut plugins = BTreeMap::from([
            ("example.root".to_owned(), plugin(&[], &[])),
            ("example.middle".to_owned(), plugin(&["example.root"], &[])),
            ("example.leaf".to_owned(), plugin(&["example.middle"], &[])),
        ]);
        let plan = plan_plugin_activation(&plugins).expect("initial plan");
        let before = plugin_activation_epochs(&plugins, &plan).expect("initial epochs");

        plugins.get_mut("example.root").expect("root").config = serde_json::json!({"revision":2});
        let plan = plan_plugin_activation(&plugins).expect("updated plan");
        let after = plugin_activation_epochs(&plugins, &plan).expect("updated epochs");

        assert_ne!(before["example.root"], after["example.root"]);
        assert_ne!(before["example.middle"], after["example.middle"]);
        assert_ne!(before["example.leaf"], after["example.leaf"]);
    }

    #[test]
    fn reload_plan_marks_the_complete_transitive_hard_closure() {
        let previous = BTreeMap::from([
            ("example.root".to_owned(), plugin(&[], &[])),
            ("example.middle".to_owned(), plugin(&["example.root"], &[])),
            ("example.leaf".to_owned(), plugin(&["example.middle"], &[])),
            ("example.unrelated".to_owned(), plugin(&[], &[])),
        ]);
        let mut current = previous.clone();
        current.get_mut("example.root").expect("root").config = serde_json::json!({"revision":2});

        let plan = plan_plugin_reload(&previous, &current).expect("reload plan");
        for plugin_id in ["example.root", "example.middle", "example.leaf"] {
            let decision = plan.decision(plugin_id).expect("decision");
            assert_eq!(decision.action, PluginReloadAction::Restart);
            assert!(
                decision
                    .reasons
                    .contains(&PluginReloadReason::DependencyEpochChanged)
            );
        }
        assert!(
            plan.decision("example.root")
                .expect("root decision")
                .triggered_by
                .is_empty()
        );
        assert_eq!(
            plan.decision("example.middle")
                .expect("middle decision")
                .triggered_by
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["example.root"]
        );
        assert_eq!(
            plan.decision("example.leaf")
                .expect("leaf decision")
                .triggered_by
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["example.middle"]
        );
        let unrelated = plan.decision("example.unrelated").expect("decision");
        assert_eq!(unrelated.action, PluginReloadAction::Restart);
        assert_eq!(unrelated.reasons, [PluginReloadReason::InProcessStatic]);
    }

    #[test]
    fn reload_plan_never_reuses_static_plugin_instances() {
        let plugins = BTreeMap::from([("example.static".to_owned(), plugin(&[], &[]))]);
        let plan = plan_plugin_reload(&plugins, &plugins).expect("reload plan");
        let decision = plan.decision("example.static").expect("decision");
        assert_eq!(decision.action, PluginReloadAction::Restart);
        assert!(
            decision
                .reasons
                .contains(&PluginReloadReason::InProcessStatic)
        );
    }
}
