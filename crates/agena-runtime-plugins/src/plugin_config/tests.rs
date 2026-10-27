use super::*;
use serde_json::{Value, json};

fn configured(value: Value) -> PluginsConfig {
    serde_json::from_value(value).unwrap()
}

fn bundled() -> BTreeMap<String, ConfiguredPlugin> {
    BTreeMap::from([(
        NOTIFICATIONS_PLUGIN.to_owned(),
        ConfiguredPlugin::static_default(),
    )])
}

#[test]
fn legacy_disabled_notification_config_retains_settings_and_timeouts() {
    let original = configured(json!({
        "list": {
            "agena.terminal": {
                "enabled": false,
                "package": {"kind": "static"},
                "settings": {"custom": {"value": 3}},
                "timeouts": {"init": "7s"}
            }
        }
    }));
    let legacy = original.list[LEGACY_NOTIFICATIONS_PLUGIN].clone();
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list.len(), 1);
    assert_eq!(merged.list[NOTIFICATIONS_PLUGIN], legacy);
    assert!(!merged.list[NOTIFICATIONS_PLUGIN].enabled);
    assert_eq!(
        merge_bundled_plugin_config(merged.clone(), bundled()).unwrap(),
        merged
    );
}

#[test]
fn explicit_canonical_notification_config_and_quota_win_over_aliases() {
    let original = configured(json!({
        "list": {
            "agena.terminal": {"enabled": false, "package": {"kind": "static"}},
            "agena.notifications": {
                "package": {"kind": "static"}, "settings": {"canonical": true}
            }
        },
        "host": {"quotas": {
            "agena.terminal": {"max_concurrent": 1},
            "agena.notifications": {"max_concurrent": 5}
        }}
    }));
    let expected = original.list[NOTIFICATIONS_PLUGIN].clone();
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list.len(), 1);
    assert_eq!(merged.list[NOTIFICATIONS_PLUGIN], expected);
    assert_eq!(merged.host.quotas.len(), 1);
    assert_eq!(
        merged.host.quotas[&NOTIFICATIONS_PLUGIN.parse::<PluginKey>().unwrap()].max_concurrent,
        5
    );
}

#[test]
fn legacy_profile_disable_applies_to_the_renamed_builtin() {
    let original = configured(json!({
        "profiles": {"quiet": {"plugins": {"agena.terminal": {"action": "disable"}}}},
        "active_profiles": ["quiet"]
    }));
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list.len(), 1);
    assert!(!merged.list[NOTIFICATIONS_PLUGIN].enabled);
    assert_eq!(
        merged.profile_resolution.changes[0].plugin_id,
        NOTIFICATIONS_PLUGIN
    );
}

#[test]
fn profile_inheritance_keeps_layer_order_across_old_and_new_names() {
    for (parent_id, child_id) in [
        (LEGACY_NOTIFICATIONS_PLUGIN, NOTIFICATIONS_PLUGIN),
        (NOTIFICATIONS_PLUGIN, LEGACY_NOTIFICATIONS_PLUGIN),
    ] {
        let original = configured(json!({
            "list": {
                "agena.terminal": {"package": {"kind": "static"}, "settings": {"base": true}}
            },
            "profiles": {
                "parent": {"plugins": {(parent_id): {"action": "disable"}}},
                "child": {
                    "extends": ["parent"],
                    "plugins": {(child_id): {
                        "action": "patch", "enabled": true, "settings_patch": {"child": true}
                    }}
                }
            },
            "active_profiles": ["child"]
        }));
        let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
        assert_eq!(merged.list.len(), 1);
        let plugin = &merged.list[NOTIFICATIONS_PLUGIN];
        assert!(plugin.enabled);
        assert_eq!(plugin.settings, json!({"base": true, "child": true}));
        assert_eq!(
            merged.profile_resolution.applied_profiles,
            ["parent", "child"]
        );
        assert!(
            merged
                .profile_resolution
                .changes
                .iter()
                .all(|change| { change.plugin_id == NOTIFICATIONS_PLUGIN })
        );
    }
}

#[test]
fn canonical_entry_wins_within_a_profile_and_legacy_removal_works() {
    let original = configured(json!({
        "profiles": {
            "quiet": {"plugins": {
                "agena.terminal": {"action": "patch", "enabled": true},
                "agena.notifications": {"action": "disable"}
            }},
            "remove": {"extends": ["quiet"], "plugins": {
                "agena.terminal": {"action": "remove"}
            }}
        },
        "active_profiles": ["quiet"]
    }));
    let merged = merge_bundled_plugin_config(original.clone(), bundled()).unwrap();
    assert!(!merged.list[NOTIFICATIONS_PLUGIN].enabled);
    assert_eq!(merged.profile_resolution.changes.len(), 1);

    let mut removed = original;
    removed.active_profiles = vec!["remove".into()];
    assert!(
        merge_bundled_plugin_config(removed, bundled())
            .unwrap()
            .list
            .is_empty()
    );
}

#[test]
fn activation_references_in_base_replacements_and_patches_and_quotas_are_migrated() {
    let activation = json!({
        "requires": ["agena.terminal", "agena.notifications", "agena.fs"],
        "after": ["agena.fs", "agena.notifications", "agena.terminal"]
    });
    let original = configured(json!({
        "list": {
            "test.base": {"package": {"kind": "static"}, "activation": activation},
            "test.patched": {"package": {"kind": "static"}}
        },
        "profiles": {"custom": {"plugins": {
            "test.replaced": {"action": "replace", "plugin": {
                "package": {"kind": "static"}, "activation": activation
            }},
            "test.patched": {"action": "patch", "activation": activation}
        }}},
        "active_profiles": ["custom"],
        "host": {"quotas": {"agena.terminal": {"rate_per_sec": 12, "burst": 20}}}
    }));
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    let expected: PluginActivationConfig = serde_json::from_value(json!({
        "requires": ["agena.notifications", "agena.fs"],
        "after": ["agena.fs", "agena.notifications"]
    }))
    .unwrap();
    for id in ["test.base", "test.replaced", "test.patched"] {
        assert_eq!(merged.list[id].activation, expected);
    }
    assert_eq!(merged.host.quotas.len(), 1);
    assert_eq!(
        merged.host.quotas[&NOTIFICATIONS_PLUGIN.parse::<PluginKey>().unwrap()].rate_per_sec,
        12
    );
}

#[test]
fn previously_resolved_profile_provenance_uses_the_new_name() {
    let mut original = configured(json!({
        "list": {"agena.terminal": {"package": {"kind": "static"}}},
        "profiles": {"quiet": {"plugins": {"agena.terminal": {"action": "disable"}}}},
        "active_profiles": ["quiet"]
    }));
    original.resolve_profiles_in_place().unwrap();
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list.len(), 1);
    assert!(!merged.list[NOTIFICATIONS_PLUGIN].enabled);
    assert_eq!(
        merged.profile_resolution.changes[0].plugin_id,
        NOTIFICATIONS_PLUGIN
    );
}

#[test]
fn configurations_without_the_bundled_notification_plugin_keep_their_ids() {
    let original = configured(json!({
        "list": {"agena.terminal": {"package": {"kind": "static"}}}
    }));
    let merged = merge_bundled_plugin_config(original.clone(), BTreeMap::new()).unwrap();
    assert_eq!(merged.list, original.list);
}
