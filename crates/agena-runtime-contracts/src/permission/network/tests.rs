use super::*;
use serde_json::{Value, json};

fn policy(egress: Value) -> NetworkPermissionConfig {
    let config: NetworkPermissionConfig = serde_json::from_value(json!({
        "egress": egress,
        "ingress": {"default": "deny", "hostLoopback": "deny"}
    }))
    .unwrap();
    config.validate().unwrap();
    config
}

fn compiled(config: &NetworkPermissionConfig) -> NetworkPermissionPolicy {
    NetworkPermissionPolicy::from_config(config).unwrap()
}

#[test]
fn full_native_request_includes_ingress_and_host_loopback_capabilities() {
    let any = "*".parse().unwrap();
    for (inbound, host_loopback, macos_supported) in [
        ("allow", "deny", true),
        ("allow", "allow", true),
        ("deny", "allow", false),
    ] {
        let config: NetworkPermissionConfig = serde_json::from_value(json!({
            "egress": {"default": "deny", "allow": [], "deny": []},
            "ingress": {"default": inbound, "hostLoopback": host_loopback}
        }))
        .unwrap();
        assert!(matches!(
            compiled(&config).check_connect(&any),
            PermissionDecision::Allow
        ));
        assert!(!allowed(
            &compiled(&config),
            "192.0.2.7",
            NetworkProtocol::Tcp,
            Some(443)
        ));
        // Windows permits these native combinations. Seatbelt requires
        // inbound allow for hostLoopback allow; Linux permits neither flag.
        config.validate_for_platform("windows").unwrap();
        assert_eq!(
            config.validate_for_platform("macos").is_ok(),
            macos_supported
        );
        assert!(config.validate_for_platform("linux").is_err());
    }
    for config in [
        NetworkPermissionConfig::offline(),
        NetworkPermissionConfig::default(),
    ] {
        assert!(matches!(
            compiled(&config).check_connect(&any),
            PermissionDecision::Deny { .. }
        ));
    }
}

#[test]
fn full_native_request_preserves_ingress_authority_ceilings() {
    let inbound: NetworkPermissionConfig = serde_json::from_value(json!({
        "egress": {"default": "deny"},
        "ingress": {"default": "allow", "hostLoopback": "allow"}
    }))
    .unwrap();
    let any = "*".parse().unwrap();
    for ceiling in [
        NetworkPermissionConfig::offline(),
        NetworkPermissionConfig::allow_all(),
    ] {
        let combined = intersect_policies(&[inbound.clone(), ceiling]).unwrap();
        assert!(matches!(
            compiled(&combined).check_connect(&any),
            PermissionDecision::Deny { .. }
        ));
        combined.validate_for_platform("linux").unwrap();
    }
    let ingress_only: NetworkPermissionConfig = serde_json::from_value(json!({
        "egress": {"default": "deny"},
        "ingress": {"default": "allow", "hostLoopback": "deny"}
    }))
    .unwrap();
    let combined = intersect_policies(&[inbound, ingress_only]).unwrap();
    assert!(matches!(
        compiled(&combined).check_connect(&any),
        PermissionDecision::Allow
    ));
    assert_eq!(
        combined.ingress.unwrap().host_loopback,
        Some(NetworkAction::Deny)
    );
}

fn distinct_native_rules(count: usize) -> Vec<NetworkRuleConfig> {
    (0..count)
        .map(|index| NetworkRuleConfig {
            to: Some(vec![NetworkPeerConfig {
                cidr: format!(
                    "{}/32",
                    std::net::Ipv4Addr::from(0x0a40_0000 + index as u32)
                ),
                except: vec![],
            }]),
            ports: Some(vec![NetworkPortConfig {
                protocol: Some(NetworkProtocol::Tcp),
                port: Some(443),
                end_port: None,
            }]),
        })
        .collect()
}

#[test]
fn single_native_policy_accepts_more_than_4096_distinct_denies() {
    let mut config = NetworkPermissionConfig::allow_all();
    config.egress.as_mut().unwrap().deny = Some(distinct_native_rules(4097));
    config.validate_for_platform("linux").unwrap();
    config.validate_for_platform("windows").unwrap();

    let result = intersect_policies(&[config]).unwrap();
    assert_eq!(
        result.egress.as_ref().unwrap().deny.as_ref().unwrap().len(),
        4097
    );
    result.validate_for_platform("linux").unwrap();
    let current = compiled(&result);
    assert!(!allowed(
        &current,
        "10.64.0.0",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(!allowed(
        &current,
        "10.64.16.0",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(allowed(
        &current,
        "10.65.0.1",
        NetworkProtocol::Tcp,
        Some(443)
    ));
}

#[test]
fn repeated_native_authority_layers_deduplicate_allows_and_denies() {
    let rules = distinct_native_rules(4097);
    let mut first = NetworkPermissionConfig::offline();
    first.egress.as_mut().unwrap().allow = Some(
        rules
            .iter()
            .cloned()
            .chain(std::iter::once(rules[0].clone()))
            .collect(),
    );
    first.egress.as_mut().unwrap().deny = Some(vec![rules[0].clone(); 4097]);
    first.validate_for_platform("linux").unwrap();
    let mut reordered = first.clone();
    reordered
        .egress
        .as_mut()
        .unwrap()
        .allow
        .as_mut()
        .unwrap()
        .reverse();

    // Repeating an authority does not create a Cartesian product. Ordering
    // and duplicate entries carry no authority in native MXC rule lists.
    let result = intersect_policies(&[
        first.clone(),
        NetworkPermissionConfig::allow_all(),
        reordered,
        first,
    ])
    .unwrap();
    let egress = result.egress.as_ref().unwrap();
    assert_eq!(egress.allow.as_ref().unwrap(), &rules);
    assert_eq!(egress.deny.as_ref().unwrap(), &vec![rules[0].clone()]);
    result.validate_for_platform("linux").unwrap();
    let current = compiled(&result);
    assert!(!allowed(
        &current,
        "10.64.0.0",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(allowed(
        &current,
        "10.64.0.1",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(!allowed(
        &current,
        "10.64.0.1",
        NetworkProtocol::Udp,
        Some(443)
    ));
}

fn allowed(
    policy: &NetworkPermissionPolicy,
    address: &str,
    protocol: NetworkProtocol,
    port: Option<u16>,
) -> bool {
    matches!(
        policy.check_endpoint(address.parse().unwrap(), protocol, port),
        PermissionDecision::Allow
    )
}

#[test]
fn explicit_denies_override_allowed_ranges() {
    let config = policy(json!({
        "default": "deny",
        "allow": [{
            "to": [{"cidr": "10.20.0.0/16"}],
            "ports": [{"protocol": "tcp", "port": 8000, "endPort": 8099}]
        }],
        "deny": [
            {
                "to": [{"cidr": "10.20.0.0/16"}],
                "ports": [{"protocol": "tcp", "port": 8080, "endPort": 8089}]
            },
            {"to": [{"cidr": "10.20.66.0/24"}]}
        ]
    }));
    let policy = compiled(&config);
    for port in [8000, 8079, 8090, 8099] {
        assert!(allowed(
            &policy,
            "10.20.1.7",
            NetworkProtocol::Tcp,
            Some(port)
        ));
    }
    for port in [7999, 8080, 8085, 8089, 8100] {
        assert!(!allowed(
            &policy,
            "10.20.1.7",
            NetworkProtocol::Tcp,
            Some(port)
        ));
    }
    assert!(!allowed(
        &policy,
        "10.20.66.7",
        NetworkProtocol::Tcp,
        Some(8001)
    ));
    assert!(!allowed(
        &policy,
        "10.20.1.7",
        NetworkProtocol::Udp,
        Some(8001)
    ));
}

#[test]
fn except_subtracts_from_its_peer_without_becoming_a_global_deny() {
    let mut config = policy(json!({
        "default": "deny",
        "allow": [{
            "to": [
                {"cidr": "10.20.0.0/16", "except": ["10.20.10.0/24"]},
                {"cidr": "10.20.10.7/32"}
            ],
            "ports": [{"protocol": "tcp", "port": 443}]
        }]
    }));
    let current = compiled(&config);
    assert!(allowed(
        &current,
        "10.20.1.7",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(allowed(
        &current,
        "10.20.10.7",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(!allowed(
        &current,
        "10.20.10.8",
        NetworkProtocol::Tcp,
        Some(443)
    ));

    config.egress.as_mut().unwrap().deny = Some(vec![
        serde_json::from_value(json!({"to": [{"cidr": "10.20.10.7/32"}]})).unwrap(),
    ]);
    assert!(!allowed(
        &compiled(&config),
        "10.20.10.7",
        NetworkProtocol::Tcp,
        Some(443)
    ));
}

#[test]
fn mapped_ipv6_rules_and_destinations_have_ipv4_wire_identity() {
    let mapped = policy(json!({
        "default": "deny",
        "allow": [{
            "to": [{
                "cidr": "::ffff:192.0.2.0/120",
                "except": ["::ffff:192.0.2.128/121"]
            }]
        }]
    }));
    let current = compiled(&mapped);
    for address in ["192.0.2.1", "::ffff:192.0.2.1"] {
        assert!(allowed(&current, address, NetworkProtocol::Tcp, Some(443)));
        assert!(allowed(&current, address, NetworkProtocol::Udp, Some(53)));
    }
    for address in ["192.0.2.200", "::ffff:192.0.2.200", "192.0.3.1"] {
        assert!(!allowed(&current, address, NetworkProtocol::Tcp, Some(443)));
    }

    let deny_mapped = compiled(&policy(json!({
        "default": "allow",
        "deny": [{"to": [{"cidr": "::ffff:192.0.2.128/121"}]}]
    })));
    assert!(!allowed(
        &deny_mapped,
        "192.0.2.200",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(!allowed(
        &deny_mapped,
        "::ffff:192.0.2.200",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    assert!(allowed(
        &deny_mapped,
        "192.0.2.1",
        NetworkProtocol::Tcp,
        Some(443)
    ));
}

#[test]
fn mapped_and_ipv4_authority_ranges_intersect_in_the_same_family() {
    let layers = [
        policy(json!({
            "default": "deny",
            "allow": [{
                "to": [{
                    "cidr": "::ffff:192.0.2.0/120",
                    "except": ["::ffff:192.0.2.128/121"]
                }]
            }]
        })),
        policy(json!({
            "default": "deny",
            "allow": [{
                "to": [{
                    "cidr": "192.0.2.0/24",
                    "except": ["192.0.2.64/26"]
                }]
            }]
        })),
    ];
    let result = intersect_policies(&layers).unwrap();
    let rules = result.egress.as_ref().unwrap().allow.as_ref().unwrap();
    assert_eq!(rules.len(), 1);
    let peer = &rules[0].to.as_ref().unwrap()[0];
    assert_eq!(peer.cidr, "192.0.2.0/24");
    assert_eq!(peer.except, ["192.0.2.128/25", "192.0.2.64/26"]);
    let result = compiled(&result);
    assert!(allowed(
        &result,
        "192.0.2.63",
        NetworkProtocol::Tcp,
        Some(443)
    ));
    for address in [
        "192.0.2.64",
        "192.0.2.127",
        "192.0.2.128",
        "::ffff:192.0.2.200",
    ] {
        assert!(!allowed(&result, address, NetworkProtocol::Tcp, Some(443)));
    }
}

#[test]
fn omitted_destinations_stay_omitted_after_policy_intersection() {
    let wildcard = policy(json!({
        "default": "deny",
        "allow": [{"ports": [{"protocol": "tcp", "port": 443}]}]
    }));
    let port_range = policy(json!({
        "default": "deny",
        "allow": [{"ports": [{"protocol": "any", "port": 440, "endPort": 450}]}]
    }));
    let result = intersect_policies(&[wildcard.clone(), port_range]).unwrap();
    result.validate_for_platform("linux").unwrap();
    let rules = result.egress.as_ref().unwrap().allow.as_ref().unwrap();
    assert_eq!(rules.len(), 1);
    assert!(
        rules[0].to.is_none(),
        "to omission must not become explicit ::/0"
    );
    let current = compiled(&result);
    for address in ["192.0.2.1", "2001:db8::1", "fd20:20::7"] {
        assert!(allowed(&current, address, NetworkProtocol::Tcp, Some(443)));
        assert!(!allowed(&current, address, NetworkProtocol::Udp, Some(443)));
    }

    let selected = policy(json!({
        "default": "deny",
        "allow": [{"to": [{"cidr": "2001:db8::/32"}]}]
    }));
    for layers in [[wildcard.clone(), selected.clone()], [selected, wildcard]] {
        let result = intersect_policies(&layers).unwrap();
        let rules = result.egress.as_ref().unwrap().allow.as_ref().unwrap();
        assert_eq!(rules[0].to.as_ref().unwrap()[0].cidr, "2001:db8::/32");
    }
}

#[test]
fn any_with_ports_matches_tcp_and_udp_but_not_icmp() {
    let dns = policy(json!({
        "default": "deny",
        "allow": [{"ports": [{"protocol": "any", "port": 53}]}]
    }));
    let current = compiled(&dns);
    assert!(allowed(
        &current,
        "192.0.2.1",
        NetworkProtocol::Tcp,
        Some(53)
    ));
    assert!(allowed(
        &current,
        "192.0.2.1",
        NetworkProtocol::Udp,
        Some(53)
    ));
    assert!(!allowed(&current, "192.0.2.1", NetworkProtocol::Icmp, None));
    assert!(!allowed(&current, "192.0.2.1", NetworkProtocol::Any, None));
    assert!(!allowed(
        &current,
        "192.0.2.1",
        NetworkProtocol::Tcp,
        Some(54)
    ));
    let icmp = policy(json!({
        "default": "deny",
        "allow": [{"ports": [{"protocol": "icmp"}]}]
    }));
    let result = intersect_policies(&[dns, icmp]).unwrap();
    assert!(result.egress.unwrap().allow.unwrap().is_empty());
}

#[test]
fn intersected_ingress_actions_require_every_authority_to_allow() {
    let left = serde_json::from_value(json!({
        "egress": {"default": "allow"},
        "ingress": {"default": "allow", "hostLoopback": "allow"}
    }))
    .unwrap();
    let right = serde_json::from_value(json!({
        "egress": {"default": "allow"},
        "ingress": {"default": "deny", "hostLoopback": "allow"}
    }))
    .unwrap();
    let result = intersect_policies(&[left, right, NetworkPermissionConfig::allow_all()]).unwrap();
    let ingress = result.ingress.unwrap();
    assert_eq!(ingress.default, Some(NetworkAction::Deny));
    assert_eq!(ingress.host_loopback, Some(NetworkAction::Deny));
}

fn policy_matrix() -> Vec<NetworkPermissionConfig> {
    vec![
        NetworkPermissionConfig::allow_all(),
        NetworkPermissionConfig::offline(),
        policy(json!({"default":"deny","allow":[{}]})),
        policy(json!({"default":"allow","deny":[{}]})),
        policy(json!({
            "default":"deny",
            "allow":[{
                "to":[
                    {"cidr":"10.20.0.0/16","except":["10.20.10.0/24"]},
                    {"cidr":"fd20:20::/48","except":["fd20:20:0:10::/64"]}
                ],
                "ports":[
                    {"protocol":"tcp","port":1000,"endPort":1010},
                    {"protocol":"udp","port":53}
                ]
            }],
            "deny":[{"to":[{"cidr":"10.20.66.0/24"}]}]
        })),
        policy(json!({
            "default":"deny",
            "allow":[
                {
                    "to":[{"cidr":"10.20.0.0/16","except":["10.20.64.0/18"]}],
                    "ports":[{"protocol":"any","port":1005,"endPort":1015}]
                },
                {
                    "to":[{"cidr":"192.0.2.0/24"}],
                    "ports":[{"protocol":"tcp","port":1,"endPort":65535}]
                }
            ]
        })),
        policy(json!({
            "default":"allow",
            "deny":[
                {"to":[{"cidr":"10.20.10.0/24"}]},
                {"ports":[{"protocol":"tcp","port":1007,"endPort":1009}]},
                {"ports":[{"protocol":"icmp"}]}
            ]
        })),
        policy(json!({
            "default":"deny",
            "allow":[{"ports":[{"protocol":"any","port":53}]}]
        })),
        policy(json!({
            "default":"deny",
            "allow":[{
                "to":[{
                    "cidr":"::ffff:192.0.2.0/120",
                    "except":["::ffff:192.0.2.128/121"]
                }]
            }]
        })),
        policy(json!({
            "default":"deny",
            "allow":[{
                "to":[{"cidr":"192.0.2.0/24","except":["192.0.2.64/26"]}]
            }]
        })),
        policy(json!({
            "default":"deny",
            "allow":[{
                "to":[{"cidr":"2001:db8::/32"},{"cidr":"fd20:20::/48"}],
                "ports":[{"protocol":"udp"},{"protocol":"icmp"}]
            }]
        })),
        policy(json!({
            "default":"deny",
            "allow":[{"ports":[{"protocol":"icmp"}]}]
        })),
        policy(json!({
            "default":"deny",
            "allow":[{
                "to":[
                    {"cidr":"10.20.0.0/16","except":["10.20.10.0/24"]},
                    {"cidr":"10.20.10.7/32"}
                ],
                "ports":[{"protocol":"tcp","port":443}]
            }]
        })),
    ]
}

fn assert_intersection_matches_all_authorities(layers: &[NetworkPermissionConfig]) {
    let result = intersect_policies(layers).unwrap();
    result.validate_for_platform("linux").unwrap();
    let result = compiled(&result);
    let layers = layers.iter().map(compiled).collect::<Vec<_>>();
    let addresses = [
        "10.20.1.7",
        "10.20.10.7",
        "10.20.10.8",
        "10.20.66.7",
        "10.21.1.7",
        "192.0.2.1",
        "192.0.2.63",
        "192.0.2.64",
        "192.0.2.127",
        "192.0.2.128",
        "::ffff:192.0.2.1",
        "::ffff:192.0.2.200",
        "2001:db8::1",
        "fd20:20::7",
        "fd20:20:0:10::7",
        "fd20:21::7",
        "127.0.0.1",
        "::1",
    ];
    let ports = [
        1, 52, 53, 54, 443, 999, 1000, 1005, 1007, 1009, 1010, 1011, 1015, 1016, 65535,
    ];
    for address in addresses {
        for protocol in [
            NetworkProtocol::Tcp,
            NetworkProtocol::Udp,
            NetworkProtocol::Icmp,
            NetworkProtocol::Any,
        ] {
            let probe_ports: Vec<Option<u16>> = match protocol {
                NetworkProtocol::Tcp | NetworkProtocol::Udp => {
                    ports.iter().copied().map(Some).collect()
                }
                _ => vec![None],
            };
            for port in probe_ports {
                let expected = layers
                    .iter()
                    .all(|layer| allowed(layer, address, protocol, port));
                assert_eq!(
                    allowed(&result, address, protocol, port),
                    expected,
                    "intersection differs from authority conjunction: {address} {protocol:?} {port:?}; {layers:?}"
                );
            }
        }
    }
}

#[test]
fn two_and_three_authority_intersections_match_a_finite_endpoint_matrix() {
    let pool = policy_matrix();
    for (left_index, left) in pool.iter().enumerate() {
        for (right_index, right) in pool.iter().enumerate() {
            assert_intersection_matches_all_authorities(&[left.clone(), right.clone()]);
            let third = &pool[(left_index * 7 + right_index) % pool.len()];
            assert_intersection_matches_all_authorities(&[
                left.clone(),
                right.clone(),
                third.clone(),
            ]);
        }
    }
}
