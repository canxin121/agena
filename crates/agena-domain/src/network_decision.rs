//! Validate remembered network decisions before they enter persistent storage.

use std::net::IpAddr;

use crate::{
    NetworkEgressConfig, NetworkPeerConfig, NetworkPermissionConfig, NetworkPortConfig,
    NetworkProtocol, NetworkRuleConfig, NetworkTarget, PermissionAction, PermissionMode,
};

/// Other permission subjects retain their own approval modes. Network decisions
/// must describe one concrete native endpoint and a concrete allow/deny action.
pub fn validate_persisted_network_decision(
    action: &PermissionAction,
    mode: PermissionMode,
    platform: &str,
) -> Result<(), String> {
    let PermissionAction::NetworkAccess { target, host, port } = action else {
        return Ok(());
    };
    if !matches!(mode, PermissionMode::Allow | PermissionMode::Deny) {
        return Err("permission rule network_access.mode must be allow or deny; native MXC cannot persist ask/auto network decisions".into());
    }
    let address = host.parse::<IpAddr>().map(canonical_ip).map_err(|_| {
        "permission rule network_access.host must be a literal IP; use permission.network CIDR rules instead of hostname or wildcard decisions".to_owned()
    })?;
    if target.trim() != target || target.contains('*') {
        return Err("permission rule network_access.target must be a concrete numeric endpoint without whitespace or wildcards".into());
    }
    let protocol = if target.contains("://") {
        let url = url::Url::parse(target).map_err(|_| {
            "permission rule network_access.target is not a valid numeric endpoint".to_owned()
        })?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err("permission rule network_access.target cannot include credentials, URL paths, queries, or fragments; native MXC controls IP addresses and ports".into());
        }
        match url.scheme() {
            "tcp" | "http" | "https" => NetworkProtocol::Tcp,
            "udp" => NetworkProtocol::Udp,
            "icmp" => NetworkProtocol::Icmp,
            _ => return Err("permission rule network_access.target supports tcp, udp, icmp, http, or https endpoint schemes".into()),
        }
    } else if port.is_some() {
        NetworkProtocol::Tcp
    } else {
        NetworkProtocol::Any
    };
    let parsed: NetworkTarget = target
        .parse()
        .map_err(|error| format!("permission rule network_access.target is invalid: {error}"))?;
    let parsed_address = parsed.host().parse::<IpAddr>().map(canonical_ip).map_err(|_| {
        "permission rule network_access.target must contain a literal IP; a hostname cannot be enforced by native MXC".to_owned()
    })?;
    if parsed_address != address || parsed.port() != *port {
        return Err("permission rule network_access target, host, and port must identify the same numeric endpoint".into());
    }
    let rule = NetworkRuleConfig {
        to: Some(vec![NetworkPeerConfig {
            cidr: format!("{address}/{}", if address.is_ipv4() { 32 } else { 128 }),
            except: Vec::new(),
        }]),
        ports: if protocol == NetworkProtocol::Any && port.is_none() {
            None
        } else {
            Some(vec![NetworkPortConfig {
                protocol: Some(protocol),
                port: *port,
                end_port: None,
            }])
        },
    };
    let egress = NetworkEgressConfig {
        allow: (mode == PermissionMode::Allow).then(|| vec![rule.clone()]),
        deny: (mode == PermissionMode::Deny).then_some(vec![rule]),
        ..Default::default()
    };
    NetworkPermissionConfig {
        egress: Some(egress),
        ingress: None,
    }
    .validate_for_platform(platform)
}

/// Storage action keys are opaque for other permission subjects. Network keys
/// are inspected so batch edits and transaction writers cannot bypass validation.
pub fn validate_persisted_network_rule_key(
    action_key: &str,
    mode: PermissionMode,
    platform: &str,
) -> Result<(), String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(action_key) else {
        return Ok(());
    };
    if value.get("kind").and_then(serde_json::Value::as_str) != Some("network_access") {
        return Ok(());
    }
    if let Some(fields) = value.as_object()
        && let Some(field) = fields
            .keys()
            .find(|field| !matches!(field.as_str(), "kind" | "target" | "host" | "port"))
    {
        return Err(format!(
            "permission rule network_access has unsupported field {field:?}; express the transport in the numeric target scheme"
        ));
    }
    let action = serde_json::from_value(value)
        .map_err(|error| format!("permission rule network_access is malformed: {error}"))?;
    validate_persisted_network_decision(&action, mode, platform)
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or(IpAddr::V6(address), IpAddr::V4),
        address => address,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(target: &str, host: &str, port: Option<u16>) -> PermissionAction {
        PermissionAction::NetworkAccess {
            target: target.into(),
            host: host.into(),
            port,
        }
    }

    #[test]
    fn native_persisted_network_endpoints_are_concrete_and_self_consistent() {
        for action in [
            action("192.0.2.7", "192.0.2.7", None),
            action("192.0.2.7:443", "192.0.2.7", Some(443)),
            action("https://192.0.2.7/", "192.0.2.7", Some(443)),
            action("udp://192.0.2.53:53", "192.0.2.53", Some(53)),
            action("icmp://192.0.2.7", "192.0.2.7", None),
            action("[2001:db8::7]:443", "2001:db8::7", Some(443)),
            action("[::ffff:192.0.2.7]:443", "192.0.2.7", Some(443)),
        ] {
            for mode in [PermissionMode::Allow, PermissionMode::Deny] {
                for platform in ["linux", "windows"] {
                    validate_persisted_network_decision(&action, mode, platform).unwrap();
                }
                assert!(validate_persisted_network_decision(&action, mode, "macos").is_err());
            }
        }
    }

    #[test]
    fn unsupported_persisted_network_rules_fail_before_storage() {
        for action in [
            action("example.com:443", "example.com", Some(443)),
            action("*", "*", None),
            action("https://192.0.2.7/private", "192.0.2.7", Some(443)),
            action("https://192.0.2.7/?token=1", "192.0.2.7", Some(443)),
            action("https://name:password@192.0.2.7/", "192.0.2.7", Some(443)),
            action("192.0.2.7:443", "192.0.2.8", Some(443)),
            action("192.0.2.7:443", "192.0.2.7", Some(80)),
            action("192.0.2.7", "192.0.2.7", Some(443)),
            action("192.0.2.7:0", "192.0.2.7", Some(0)),
            action("icmp://192.0.2.7:53", "192.0.2.7", Some(53)),
        ] {
            assert!(
                validate_persisted_network_decision(&action, PermissionMode::Deny, "linux")
                    .is_err(),
                "{action:?}"
            );
        }
        let action = action("192.0.2.7:443", "192.0.2.7", Some(443));
        for mode in [PermissionMode::Ask, PermissionMode::Auto] {
            assert!(validate_persisted_network_decision(&action, mode, "linux").is_err());
        }
    }

    #[test]
    fn persisted_network_key_validation_does_not_change_other_subjects() {
        validate_persisted_network_rule_key("opaque-tool-key", PermissionMode::Ask, "macos")
            .unwrap();
        let tool = serde_json::to_string(&PermissionAction::Tool {
            tool_name: "agena.shell.exec".into(),
            qualifier: None,
        })
        .unwrap();
        validate_persisted_network_rule_key(&tool, PermissionMode::Auto, "macos").unwrap();
        assert!(
            validate_persisted_network_rule_key(
                r#"{"kind":"network_access","host":"192.0.2.7"}"#,
                PermissionMode::Deny,
                "linux",
            )
            .is_err()
        );
        assert!(validate_persisted_network_rule_key(
            r#"{"kind":"network_access","target":"192.0.2.7:53","host":"192.0.2.7","port":53,"protocol":"udp"}"#,
            PermissionMode::Deny,
            "linux",
        ).is_err());
    }
}
