use super::*;

fn denied(decision: PermissionDecision) -> bool {
    matches!(decision, PermissionDecision::Deny { .. })
}

#[test]
fn ipv6_urls_and_mapped_loopback_targets_cannot_skip_the_loopback_default() {
    let mut policy = NetworkPermissionPolicy::allow_all();
    policy.loopback_default = PermissionMode::Deny;
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
fn exact_and_cidr_mapped_rules_match_the_same_ipv4_destinations() {
    for rule in ["127.0.0.1:443", "[::ffff:127.0.0.1]:443"] {
        let mut policy = NetworkPermissionPolicy::allow_all();
        policy.add_rule(rule, PermissionMode::Deny).unwrap();
        for target in [
            "127.0.0.1:443",
            "[::ffff:7f00:1]:443",
            "https://[::ffff:127.0.0.1]/",
        ] {
            assert!(
                denied(policy.check_connect(&target.parse().unwrap())),
                "{rule} / {target}"
            );
        }
    }
    let mut policy = NetworkPermissionPolicy::allow_all();
    policy
        .add_rule("[::ffff:127.0.0.0/104]:443", PermissionMode::Deny)
        .unwrap();
    for target in ["127.32.1.9:443", "[::ffff:127.32.1.9]:443"] {
        assert!(
            denied(policy.check_connect(&target.parse().unwrap())),
            "{target}"
        );
    }
    for target in ["128.32.1.9:443", "127.32.1.9:80"] {
        assert!(matches!(
            policy.check_connect(&target.parse().unwrap()),
            PermissionDecision::Allow
        ));
    }
    for rule in ["::ffff:127.0.0.1/95", "[::ffff:127.0.0.0/64]:443"] {
        assert!(
            policy.add_rule(rule, PermissionMode::Allow).is_err(),
            "{rule}"
        );
    }
}

#[test]
fn ordinary_ipv6_cidrs_keep_their_native_address_family() {
    let mut policy = NetworkPermissionPolicy::new(PermissionMode::Deny);
    policy.add_rule("::/0", PermissionMode::Allow).unwrap();
    assert!(matches!(
        policy.check_connect(&"[2001:db8::1]:443".parse().unwrap()),
        PermissionDecision::Allow
    ));
    for target in ["8.8.8.8:443", "[::ffff:8.8.8.8]:443"] {
        assert!(
            denied(policy.check_connect(&target.parse().unwrap())),
            "{target}"
        );
    }
}

#[test]
fn resolved_dns_address_checks_keep_mapped_ip_fences() {
    let mut network = NetworkPermissionPolicy::allow_all();
    network
        .add_rule("[::ffff:127.0.0.0/104]:443", PermissionMode::Deny)
        .unwrap();
    network
        .add_rule("example.com:443", PermissionMode::Allow)
        .unwrap();
    let snapshot = SandboxPermissionSnapshot::from_policies(
        false,
        Path::new(""),
        std::iter::empty(),
        std::iter::once(&network),
    );
    for resolved in ["127.0.0.1:443", "[::ffff:127.0.0.1]:443"] {
        assert!(
            denied(snapshot.authorize_resolved_network_address(
                &"example.com:443".parse().unwrap(),
                &resolved.parse().unwrap(),
            )),
            "{resolved}"
        );
    }
}
