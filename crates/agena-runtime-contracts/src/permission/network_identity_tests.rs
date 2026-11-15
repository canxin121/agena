use super::*;
use serde_json::json;

fn denied(decision: PermissionDecision) -> bool {
    matches!(decision, PermissionDecision::Deny { .. })
}

fn native_policy(value: serde_json::Value) -> NetworkPermissionPolicy {
    NetworkPermissionPolicy::from_config(&serde_json::from_value(value).unwrap()).unwrap()
}

#[test]
fn ipv6_urls_and_mapped_loopback_targets_cannot_skip_native_host_loopback_denial() {
    let policy = NetworkPermissionPolicy::allow_all();
    for target in [
        "http://[::1]/",
        "[::1]:80",
        "http://[::ffff:127.0.0.1]/",
        "[::ffff:7f00:1]:80",
        "127.0.0.1:80",
    ] {
        assert!(
            denied(policy.check_connect(&target.parse().unwrap())),
            "{target}"
        );
    }
}

#[test]
fn ipv4_cidr_denials_cover_mapped_ipv6_target_spellings() {
    let policy = native_policy(json!({
        "egress": {
            "default":"allow",
            "deny":[{
                "to":[{"cidr":"192.0.2.0/24"}],
                "ports":[{"protocol":"tcp","port":443}]
            }]
        }
    }));
    for target in [
        "192.0.2.7:443",
        "[::ffff:192.0.2.7]:443",
        "https://[::ffff:c000:207]/",
    ] {
        assert!(
            denied(policy.check_connect(&target.parse().unwrap())),
            "{target}"
        );
    }
    for target in ["192.0.3.7:443", "192.0.2.7:80"] {
        assert!(matches!(
            policy.check_connect(&target.parse().unwrap()),
            PermissionDecision::Allow
        ));
    }
}

#[test]
fn mapped_ipv6_cidr_denials_have_the_same_ipv4_native_meaning() {
    let policy = native_policy(json!({
        "egress":{"default":"allow","deny":[{
            "to":[{"cidr":"::ffff:192.0.2.0/120","except":["::ffff:192.0.2.128/121"]}]
        }]}
    }));
    for target in ["192.0.2.7:443", "[::ffff:192.0.2.7]:443"] {
        assert!(
            denied(policy.check_connect(&target.parse().unwrap())),
            "{target}"
        );
    }
    assert!(matches!(
        policy.check_connect(&"192.0.2.200:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
}

#[test]
fn ordinary_ipv6_cidrs_keep_their_native_address_family() {
    let policy = native_policy(json!({
        "egress":{"default":"deny","allow":[{"to":[{"cidr":"2001:db8::/32"}]}]}
    }));
    assert!(matches!(
        policy.check_connect(&"[2001:db8::1]:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
    for target in ["192.0.2.7:443", "[::ffff:192.0.2.7]:443"] {
        assert!(
            denied(policy.check_connect(&target.parse().unwrap())),
            "{target}"
        );
    }
}

#[test]
fn hostnames_require_resolution_and_resolved_addresses_keep_native_denials() {
    let network = native_policy(json!({
        "egress":{"default":"allow","deny":[{"to":[{"cidr":"192.0.2.0/24"}]}]}
    }));
    assert!(denied(
        network.check_connect(&"example.com:443".parse().unwrap())
    ));
    let snapshot = SandboxPermissionSnapshot::from_policies(
        false,
        Path::new(""),
        std::iter::empty(),
        std::iter::once(&network),
    );
    for resolved in ["192.0.2.7:443", "[::ffff:192.0.2.7]:443"] {
        assert!(
            denied(snapshot.authorize_resolved_network_address(
                &"example.com:443".parse().unwrap(),
                &resolved.parse().unwrap(),
            )),
            "{resolved}"
        );
    }
    assert!(matches!(
        snapshot.authorize_resolved_network_address(
            &"example.com:443".parse().unwrap(),
            &"198.51.100.7:443".parse().unwrap(),
        ),
        PermissionDecision::Allow
    ));
}
