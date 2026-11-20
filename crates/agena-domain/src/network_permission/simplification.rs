//! Lossless lowering of authored policies before native capability checks.

use super::{NetworkAction, NetworkPermissionConfig, NetworkProtocol, NetworkRuleConfig};

impl NetworkPermissionConfig {
    /// Validate every authored selector, then remove rules with no effect on access.
    ///
    /// Keep this separate from serialization so users retain their editable rules.
    /// Native adapters and their capability/budget checks must use the same result.
    pub fn simplified_for_sandbox(&self) -> Result<Self, String> {
        self.validate()?;
        let mut policy = self.resolved();
        let egress = policy.egress.as_mut().expect("resolved egress");
        let allow = egress.allow.as_mut().expect("resolved allow rules");
        let deny = egress.deny.as_mut().expect("resolved deny rules");

        // Denies take precedence, including when an allow also covers everything.
        if deny.iter().any(matches_every_endpoint) {
            egress.default = Some(NetworkAction::Deny);
            allow.clear();
            deny.clear();
        } else if egress.default == Some(NetworkAction::Allow)
            || allow.iter().any(matches_every_endpoint)
        {
            egress.default = Some(NetworkAction::Allow);
            allow.clear();
        } else if allow.is_empty() {
            // Default deny with no grants already rejects every outbound endpoint.
            deny.clear();
        }
        Ok(policy)
    }
}

fn matches_every_endpoint(rule: &NetworkRuleConfig) -> bool {
    rule.to.is_none()
        && rule.ports.as_ref().is_none_or(|ports| {
            ports.iter().any(|selector| {
                selector.protocol.unwrap_or_default() == NetworkProtocol::Any
                    && selector.port.is_none()
                    && selector.end_port.is_none()
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NetworkPeerConfig, NetworkPortConfig};
    use serde_json::{Value, json};

    fn policy(value: Value) -> NetworkPermissionConfig {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn redundant_rules_and_unconditional_rules_lower_to_defaults_on_every_platform() {
        for (authored, expected) in [
            (
                json!({"default":"allow","allow":[{"to":"192.0.2.0/24","protocol":"tcp","ports":[443]}]}),
                NetworkAction::Allow,
            ),
            (
                json!({"default":"deny","deny":[{"to":"192.0.2.0/24","protocol":"udp","ports":[53]}]}),
                NetworkAction::Deny,
            ),
            (
                json!({"default":"deny","allow":[{"to":"*"}]}),
                NetworkAction::Allow,
            ),
            (
                json!({"allow":[{"to":"*","protocol":"any","except":[]}]}),
                NetworkAction::Allow,
            ),
            (
                json!({"default":"allow","deny":[{"to":"*"}]}),
                NetworkAction::Deny,
            ),
            (
                json!({"allow":[{"to":"*"}],"deny":[{"to":"*"}]}),
                NetworkAction::Deny,
            ),
            (
                json!({"allow":[{"to":"192.0.2.9","ports":[443]}],"deny":[{"to":"*"}]}),
                NetworkAction::Deny,
            ),
        ] {
            let config = policy(authored);
            let original = serde_json::to_value(&config).unwrap();
            let lowered = config.simplified_for_sandbox().unwrap();
            let egress = lowered.egress.as_ref().unwrap();
            assert_eq!(egress.default, Some(expected));
            assert!(egress.allow.as_ref().unwrap().is_empty());
            assert!(egress.deny.as_ref().unwrap().is_empty());
            for platform in ["linux", "macos", "windows"] {
                config.validate_for_platform(platform).unwrap();
            }
            assert_eq!(serde_json::to_value(&config).unwrap(), original);
            assert_eq!(lowered.simplified_for_sandbox().unwrap(), lowered);
        }
    }

    #[test]
    fn effective_restrictions_are_retained_after_removing_redundant_grants() {
        let denied = json!([{"to":"192.0.2.0/24","except":["192.0.2.64/26"],"protocol":"tcp","ports":[443]}]);
        for config in [
            policy(json!({"default":"allow","allow":[{"to":"192.0.2.99"}],"deny":denied})),
            policy(json!({"allow":[{"to":"*"}],"deny":denied})),
        ] {
            let lowered = config.simplified_for_sandbox().unwrap();
            assert_eq!(
                serde_json::to_value(&lowered).unwrap(),
                json!({"version":2,"default":"allow","allow":[],"deny":denied,"inbound":"deny","hostLoopback":"deny"})
            );
            assert!(config.validate_for_platform("macos").is_err());
            config.validate_for_platform("linux").unwrap();
            config.validate_for_platform("windows").unwrap();
        }
    }

    #[test]
    fn wildcard_destinations_with_protocol_or_port_limits_are_not_full_access() {
        for rule in [
            json!({"to":"*","protocol":"tcp"}),
            json!({"to":"*","protocol":"udp"}),
            json!({"to":"*","protocol":"icmp"}),
            json!({"to":"*","protocol":"any","ports":["1-65535"]}),
            json!({"to":"0.0.0.0/0"}),
            json!({"to":"2001:db8::/32"}),
            json!({"to":"192.0.2.0/24","except":["192.0.2.64/26"]}),
        ] {
            for (default, kind) in [("deny", "allow"), ("allow", "deny")] {
                let mut authored = json!({"default":default});
                authored[kind] = json!([rule]);
                let config = policy(authored);
                assert_eq!(config.simplified_for_sandbox().unwrap(), config);
                assert!(config.validate_for_platform("macos").is_err());
            }
        }
    }

    #[test]
    fn explicit_native_any_selector_without_ports_is_unconditional() {
        let mut config = NetworkPermissionConfig::offline();
        config.egress.as_mut().unwrap().allow = Some(vec![NetworkRuleConfig {
            to: None,
            ports: Some(vec![NetworkPortConfig::default()]),
        }]);
        assert_eq!(
            config.simplified_for_sandbox().unwrap(),
            NetworkPermissionConfig::allow_all()
        );
    }

    #[test]
    fn invalid_authored_rules_are_rejected_even_if_they_would_be_redundant() {
        let mut invalid = NetworkPermissionConfig::allow_all();
        invalid.egress.as_mut().unwrap().allow = Some(vec![NetworkRuleConfig {
            to: Some(vec![NetworkPeerConfig {
                cidr: "192.0.2.1/24".into(),
                except: Vec::new(),
            }]),
            ports: None,
        }]);
        let mut invalid_port = NetworkPermissionConfig::allow_all();
        invalid_port.egress.as_mut().unwrap().allow = Some(vec![NetworkRuleConfig {
            to: None,
            ports: Some(vec![NetworkPortConfig {
                port: Some(0),
                ..Default::default()
            }]),
        }]);
        let mut invalid_under_deny = invalid_port.clone();
        invalid_under_deny.egress.as_mut().unwrap().deny = Some(vec![NetworkRuleConfig::default()]);
        for config in [invalid, invalid_port, invalid_under_deny] {
            assert!(config.simplified_for_sandbox().is_err());
            for platform in ["linux", "macos", "windows"] {
                assert!(config.validate_for_platform(platform).is_err());
            }
        }
    }

    #[test]
    fn ingress_and_loopback_capabilities_are_not_relaxed_by_egress_simplification() {
        for value in [
            json!({"allow":[{"to":"*"}],"inbound":"allow","hostLoopback":"deny"}),
            json!({"deny":[{"to":"*"}],"inbound":"allow","hostLoopback":"allow"}),
        ] {
            let config = policy(value);
            assert_eq!(
                config.simplified_for_sandbox().unwrap().ingress,
                config.ingress
            );
            config.validate_for_platform("macos").unwrap();
            assert!(config.validate_for_platform("linux").is_err());
        }
        let invalid = policy(json!({"allow":[{"to":"*"}],"hostLoopback":"allow"}));
        assert!(invalid.validate_for_platform("macos").is_err());
    }

    #[test]
    fn native_budgets_and_mapped_address_checks_apply_only_to_effective_rules() {
        let mut oversized = NetworkPermissionConfig::allow_all();
        oversized.egress.as_mut().unwrap().allow = Some(vec![NetworkRuleConfig {
            to: None,
            ports: Some(vec![
                NetworkPortConfig {
                    protocol: Some(NetworkProtocol::Any),
                    port: Some(443),
                    end_port: None,
                };
                16_384
            ]),
        }]);
        oversized.validate_for_platform("linux").unwrap();
        oversized.egress.as_mut().unwrap().default = Some(NetworkAction::Deny);
        assert!(
            oversized
                .validate_for_platform("linux")
                .unwrap_err()
                .contains("65536 firewall rules")
        );
        for config in [
            policy(json!({"default":"allow","allow":[{"to":"::/0"}]})),
            policy(json!({"default":"deny","deny":[{"to":"::/0"}]})),
        ] {
            config.validate_for_platform("linux").unwrap();
        }
        assert!(
            policy(json!({"default":"deny","allow":[{"to":"::/0"}]}))
                .validate_for_platform("linux")
                .unwrap_err()
                .contains("IPv4-mapped")
        );
    }
}
