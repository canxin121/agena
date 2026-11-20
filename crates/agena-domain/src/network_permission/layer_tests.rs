use super::*;
use serde_json::{Value, json};

fn network(value: Value) -> NetworkPermissionConfig {
    serde_json::from_value(value).unwrap()
}

#[test]
fn version_two_round_trips_missing_fields_and_explicit_deny_or_empty_lists() {
    for document in [
        json!({"version":2}),
        json!({"version":2,"default":"deny"}),
        json!({"version":2,"inbound":"deny"}),
        json!({"version":2,"hostLoopback":"deny"}),
        json!({"version":2,"allow":[]}),
        json!({"version":2,"deny":[]}),
    ] {
        assert_eq!(
            serde_json::to_value(network(document.clone())).unwrap(),
            document
        );
    }
    let empty: crate::PermissionConfig =
        serde_json::from_value(json!({"network":{"version":2}})).unwrap();
    assert!(empty.is_empty());
    let explicit: crate::PermissionConfig =
        serde_json::from_value(json!({"network":{"version":2,"allow":[]}})).unwrap();
    assert!(!explicit.is_empty());
    for invalid in [
        json!({"version":3}),
        json!({"version":null}),
        json!({"version":2,"default":false}),
        json!({"version":2,"allow":null}),
    ] {
        assert!(serde_json::from_value::<NetworkPermissionConfig>(invalid).is_err());
    }
}

#[test]
fn each_authored_network_field_overrides_only_itself() {
    let parent = json!({
        "version":2,"default":"allow","inbound":"allow","hostLoopback":"allow",
        "allow":[{"to":"192.0.2.10","ports":[443]}],
        "deny":[{"to":"10.0.0.0/8"}],
    });
    for (field, value) in [
        ("default", json!("deny")),
        ("inbound", json!("deny")),
        ("hostLoopback", json!("deny")),
        ("allow", json!([])),
        ("deny", json!([])),
    ] {
        let mut overlay = json!({"version":2});
        overlay[field] = value.clone();
        let mut merged = network(parent.clone());
        merged.merge_from(network(overlay));
        let mut expected = parent.clone();
        expected[field] = value;
        assert_eq!(serde_json::to_value(merged).unwrap(), expected, "{field}");
    }
    let mut unchanged = network(parent.clone());
    unchanged.merge_from(NetworkPermissionConfig::default());
    assert_eq!(serde_json::to_value(unchanged).unwrap(), parent);
}

#[test]
fn legacy_partial_blocks_pin_all_historical_defaults_before_layering() {
    let parent = network(
        json!({"default":"allow","inbound":"allow","hostLoopback":"allow","allow":[{"to":"*"}],"deny":[{"to":"10.0.0.0/8"}]}),
    );
    for legacy in [json!({}), json!({"inbound":"allow"}), json!({"allow":[]})] {
        let overlay = network(legacy.clone());
        let canonical = serde_json::to_value(&overlay).unwrap();
        assert_eq!(canonical["version"], 2);
        assert_eq!(canonical["default"], "deny");
        assert_eq!(canonical["allow"], json!([]));
        assert_eq!(canonical["deny"], json!([]));
        assert_eq!(
            canonical["inbound"],
            legacy.get("inbound").cloned().unwrap_or(json!("deny"))
        );
        assert_eq!(canonical["hostLoopback"], "deny");
        let mut merged = parent.clone();
        merged.merge_from(overlay.clone());
        assert_eq!(merged, overlay);
    }
}

#[test]
fn aggregate_permission_merging_resolves_v2_before_execution_defaults() {
    let mut base = crate::PermissionConfig::global_default();
    base.merge_from(
        serde_json::from_value(json!({"network":{"version":2,"hostLoopback":"allow"}})).unwrap(),
    );
    let resolved = base.network.unwrap().resolved();
    assert_eq!(
        resolved.egress.as_ref().unwrap().default,
        Some(NetworkAction::Allow)
    );
    assert_eq!(
        resolved.ingress.as_ref().unwrap().default,
        Some(NetworkAction::Deny)
    );
    assert_eq!(
        resolved.ingress.as_ref().unwrap().host_loopback,
        Some(NetworkAction::Allow)
    );
}
