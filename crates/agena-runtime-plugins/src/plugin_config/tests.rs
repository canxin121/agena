use super::*;
use agena_plugin_host::sdk::PluginKey;
use serde_json::{Value, json};

const NOTIFICATIONS_PLUGIN: &str = "agena.notifications";

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
fn explicit_builtin_settings_and_disabled_state_override_bundled_defaults() {
    let original = configured(json!({
        "list": {
            "agena.notifications": {
                "enabled": false, "package": {"kind": "static"},
                "settings": {"custom": {"value": 3}}, "timeouts": {"init": "7s"}
            }
        },
        "host": {"quotas": {"agena.notifications": {"max_concurrent": 5}}}
    }));
    let expected = original.list[NOTIFICATIONS_PLUGIN].clone();
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list.len(), 1);
    assert_eq!(merged.list[NOTIFICATIONS_PLUGIN], expected);
    assert_eq!(
        merged.host.quotas[&NOTIFICATIONS_PLUGIN.parse::<PluginKey>().unwrap()].max_concurrent,
        5
    );
    assert_eq!(
        merge_bundled_plugin_config(merged.clone(), bundled()).unwrap(),
        merged
    );
}

#[test]
fn profile_inheritance_preserves_disable_patch_and_provenance_order() {
    let original = configured(json!({
        "list": {"agena.notifications": {
            "package": {"kind":"static"}, "settings": {"base": true}
        }},
        "profiles": {
            "parent": {"plugins": {"agena.notifications": {"action":"disable"}}},
            "child": {"extends":["parent"], "plugins": {"agena.notifications": {
                "action":"patch", "enabled":true, "settings_patch":{"child":true}
            }}}
        },
        "active_profiles":["child"]
    }));
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list.len(), 1);
    assert!(merged.list[NOTIFICATIONS_PLUGIN].enabled);
    assert_eq!(
        merged.list[NOTIFICATIONS_PLUGIN].settings,
        json!({"base":true,"child":true})
    );
    assert_eq!(
        merged.profile_resolution.applied_profiles,
        ["parent", "child"]
    );
    assert!(
        merged
            .profile_resolution
            .changes
            .iter()
            .all(|change| change.plugin_id == NOTIFICATIONS_PLUGIN)
    );
}

#[test]
fn profiles_remove_real_entries_and_reject_missing_builtin_targets() {
    let original = configured(json!({
        "profiles":{"remove":{"plugins":{"agena.notifications":{"action":"remove"}}}},
        "active_profiles":["remove"]
    }));
    assert!(
        merge_bundled_plugin_config(original, bundled())
            .unwrap()
            .list
            .is_empty()
    );
    let retired = configured(json!({
        "profiles":{"old":{"plugins":{"agena.snapshot":{"action":"disable"}}}},
        "active_profiles":["old"]
    }));
    let error = merge_bundled_plugin_config(retired, bundled()).unwrap_err();
    assert!(error.contains("cannot disable missing plugin `agena.snapshot`"));
}

#[test]
fn current_activation_dependencies_and_external_entries_are_preserved() {
    let activation = json!({"requires":["agena.notifications"],"after":["agena.fs"]});
    let original = configured(json!({
        "list": {"test.base":{"package":{"kind":"static"},"activation":activation}},
        "profiles":{"custom":{"plugins":{
            "test.replaced":{"action":"replace","plugin":{"package":{"kind":"static"},"activation":activation}}
        }}},
        "active_profiles":["custom"]
    }));
    let expected = original.list["test.base"].activation.clone();
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list["test.base"].activation, expected);
    assert_eq!(merged.list["test.replaced"].activation, expected);
    assert_eq!(merged.list.len(), 3);
}

#[test]
fn configuration_does_not_rewrite_plugin_identifiers_or_inject_retired_entries() {
    let original = configured(json!({"list":{
        "agena.terminal":{"enabled":false,"package":{"kind":"static"}}
    }}));
    let expected = original.list["agena.terminal"].clone();
    let merged = merge_bundled_plugin_config(original, bundled()).unwrap();
    assert_eq!(merged.list["agena.terminal"], expected);
    assert!(merged.list[NOTIFICATIONS_PLUGIN].enabled);
    assert!(!merged.list.contains_key("agena.snapshot"));
}
