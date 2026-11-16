//! Flat network settings compiled to the native MXC network model.

mod flat;
mod simplification;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

/// Network decisions are concrete sandbox permissions; approval belongs to tools.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkAction {
    Allow,
    #[default]
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkProtocol {
    Tcp,
    Udp,
    Icmp,
    #[default]
    Any,
}

/// Native composition model with a flat, human-editable serialized format.
/// A present network section is a complete policy; missing defaults deny.
#[derive(Debug, Clone, Default)]
pub struct NetworkPermissionConfig {
    pub egress: Option<NetworkEgressConfig>,
    pub ingress: Option<NetworkIngressConfig>,
}

impl Serialize for NetworkPermissionConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        flat::NetworkDocument::from_native(self)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NetworkPermissionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        flat::NetworkDocument::deserialize(deserializer)?
            .into_native()
            .map_err(serde::de::Error::custom)
    }
}

// Native composition can group several peers/protocols into one rule. Its flat
// transport expands those groups, so policy identity must survive that expansion.
impl PartialEq for NetworkPermissionConfig {
    fn eq(&self, other: &Self) -> bool {
        match (
            flat::NetworkDocument::from_native(self),
            flat::NetworkDocument::from_native(other),
        ) {
            (Ok(left), Ok(right)) => left == right,
            _ => self.egress == other.egress && self.ingress == other.ingress,
        }
    }
}

impl Eq for NetworkPermissionConfig {}

impl std::hash::Hash for NetworkPermissionConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match flat::NetworkDocument::from_native(self) {
            Ok(document) => {
                0u8.hash(state);
                document.hash(state);
            }
            Err(_) => {
                1u8.hash(state);
                self.egress.hash(state);
                self.ingress.hash(state);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkEgressConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub default: Option<NetworkAction>,
    /// Explicit native grants. Public settings replace the complete network block.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub allow: Option<Vec<NetworkRuleConfig>>,
    /// Denies take precedence over allows, independent of array order.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub deny: Option<Vec<NetworkRuleConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkIngressConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub default: Option<NetworkAction>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub host_loopback: Option<NetworkAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkRuleConfig {
    /// Omitted destinations match both IP families. An explicit list is nonempty.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub to: Option<Vec<NetworkPeerConfig>>,
    /// Omitted selectors match all protocols and ports. An explicit list is nonempty.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub ports: Option<Vec<NetworkPortConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkPeerConfig {
    pub cidr: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkPortConfig {
    /// Omitted protocol matches any protocol.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub protocol: Option<NetworkProtocol>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub port: Option<u16>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    pub end_port: Option<u16>,
}

// MXC distinguishes omitted fields from explicit null. Keep Option for config
// inheritance, but require every authored value to deserialize as its real type.
fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub(crate) fn deserialize_network<'de, D>(
    deserializer: D,
) -> Result<Option<NetworkPermissionConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_path_to_error::deserialize::<_, NetworkPermissionConfig>(deserializer)
        .map(Some)
        .map_err(|error| {
            let path = error.path().to_string();
            let field = if path == "." {
                "permission.network".to_string()
            } else {
                format!("permission.network.{path}")
            };
            serde::de::Error::custom(format!("{field}: {}", error.inner()))
        })
}

impl NetworkPermissionConfig {
    /// An explicit offline policy replaces any inherited network policy.
    pub fn offline() -> Self {
        Self::default().resolved()
    }

    /// Permit outbound connections while retaining denied inbound/host-loopback.
    pub fn allow_all() -> Self {
        let mut policy = Self::offline();
        policy.egress.as_mut().expect("resolved egress").default = Some(NetworkAction::Allow);
        policy
    }

    /// Fill missing effective defaults without changing selector semantics.
    pub fn resolved(&self) -> Self {
        let egress = self.egress.clone().unwrap_or_default();
        let ingress = self.ingress.clone().unwrap_or_default();
        Self {
            egress: Some(NetworkEgressConfig {
                default: Some(egress.default.unwrap_or_default()),
                allow: Some(egress.allow.unwrap_or_default()),
                deny: Some(egress.deny.unwrap_or_default()),
            }),
            ingress: Some(NetworkIngressConfig {
                default: Some(ingress.default.unwrap_or_default()),
                host_loopback: Some(ingress.host_loopback.unwrap_or_default()),
            }),
        }
    }

    /// A present network block replaces the complete configuration.
    pub fn merge_from(&mut self, overlay: Self) {
        *self = overlay;
    }

    /// Decode the retired nested shape at an explicit file-migration boundary.
    /// Ordinary settings/API deserialization accepts only the flat format.
    pub fn from_legacy_value(value: serde_json::Value) -> Result<Self, String> {
        serde_path_to_error::deserialize::<_, flat::LegacyNetworkDocument>(value)
            .map(flat::LegacyNetworkDocument::into_native)
            .map_err(|error| format!("permission.network.{}: {}", error.path(), error.inner()))
    }

    /// Resolve old partial layers before converting them to complete flat blocks.
    pub fn merge_legacy_from(&mut self, overlay: Self) {
        if let Some(overlay) = overlay.egress {
            let current = self.egress.get_or_insert_with(Default::default);
            if overlay.default.is_some() {
                current.default = overlay.default;
            }
            if overlay.allow.is_some() {
                current.allow = overlay.allow;
            }
            if overlay.deny.is_some() {
                current.deny = overlay.deny;
            }
        }
        if let Some(overlay) = overlay.ingress {
            let current = self.ingress.get_or_insert_with(Default::default);
            if overlay.default.is_some() {
                current.default = overlay.default;
            }
            if overlay.host_loopback.is_some() {
                current.host_loopback = overlay.host_loopback;
            }
        }
    }

    /// Validate the shared native contract. Backend capability checks are separate.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(egress) = &self.egress {
            for (kind, rules) in [("allow", &egress.allow), ("deny", &egress.deny)] {
                if let Some(rules) = rules {
                    for (index, rule) in rules.iter().enumerate() {
                        rule.validate(&format!("permission.network.{kind}[{index}]"))?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl NetworkPermissionConfig {
    /// Reject native policies the selected OS backend cannot enforce.
    pub fn validate_for_platform(&self, platform: &str) -> Result<(), String> {
        let policy = self.simplified_for_sandbox()?;
        let ingress = policy.ingress.as_ref();
        let inbound = ingress.and_then(|value| value.default).unwrap_or_default();
        let loopback = ingress
            .and_then(|value| value.host_loopback)
            .unwrap_or_default();
        match platform {
            "linux" => {
                if let Some(egress) = &policy.egress {
                    for (kind, rules) in [("allow", &egress.allow), ("deny", &egress.deny)] {
                        for (rule_index, rule) in rules.iter().flatten().enumerate() {
                            for (peer_index, peer) in rule.to.iter().flatten().enumerate() {
                                let field = format!(
                                    "permission.network.{kind}[{rule_index}].to[{peer_index}]"
                                );
                                validate_bubblewrap_cidr(&peer.cidr, &format!("{field}.cidr"))?;
                                for (index, except) in peer.except.iter().enumerate() {
                                    validate_bubblewrap_cidr(
                                        except,
                                        &format!("{field}.except[{index}]"),
                                    )?;
                                }
                            }
                        }
                    }
                }
                if inbound == NetworkAction::Allow {
                    return Err(
                        "permission.network.inbound: Linux Bubblewrap does not support allow"
                            .into(),
                    );
                }
                if loopback == NetworkAction::Allow {
                    return Err(
                        "permission.network.hostLoopback: Linux Bubblewrap does not support allow"
                            .into(),
                    );
                }
                if let Some(egress) = &policy.egress {
                    validate_bubblewrap_budget(egress)?;
                }
            }
            "macos" => {
                if let Some(egress) = &policy.egress {
                    for (field, rules) in [("allow", &egress.allow), ("deny", &egress.deny)] {
                        if rules.as_ref().is_some_and(|rules| !rules.is_empty()) {
                            return Err(format!(
                                "permission.network.{field}: macOS Seatbelt does not support CIDR, protocol, or port rules"
                            ));
                        }
                    }
                }
                if loopback == NetworkAction::Allow && inbound != NetworkAction::Allow {
                    return Err("permission.network.hostLoopback: macOS Seatbelt requires inbound=allow when hostLoopback=allow".into());
                }
            }
            "windows" => {}
            _ => {
                return Err(format!(
                    "permission.network: native MXC is unavailable on platform {platform}"
                ));
            }
        }
        Ok(())
    }
}

impl NetworkEgressConfig {
    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.allow.is_none() && self.deny.is_none()
    }
}

impl NetworkIngressConfig {
    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.host_loopback.is_none()
    }
}

impl NetworkRuleConfig {
    fn validate(&self, field: &str) -> Result<(), String> {
        if let Some(peers) = &self.to {
            if peers.is_empty() {
                return Err(format!(
                    "{field}.to must be nonempty; omit it to match all destinations"
                ));
            }
            for (index, peer) in peers.iter().enumerate() {
                let field = format!("{field}.to[{index}]");
                let cidr = parse_cidr(&peer.cidr, &format!("{field}.cidr"))?;
                for (index, excluded) in peer.except.iter().enumerate() {
                    let field = format!("{field}.except[{index}]");
                    let excluded = parse_cidr(excluded, &field)?;
                    if !cidr.contains(&excluded) {
                        return Err(format!("{field} must be a subnet within {}", peer.cidr));
                    }
                }
            }
        }
        if let Some(ports) = &self.ports {
            if ports.is_empty() {
                return Err(format!(
                    "{field}.ports must be nonempty; omit it to match all protocols and ports"
                ));
            }
            for (index, selector) in ports.iter().enumerate() {
                let field = format!("{field}.ports[{index}]");
                if selector.port == Some(0) || selector.end_port == Some(0) {
                    return Err(format!("{field} ports must be between 1 and 65535"));
                }
                if let Some(end) = selector.end_port {
                    let Some(start) = selector.port else {
                        return Err(format!("{field}.endPort requires port"));
                    };
                    if end < start {
                        return Err(format!(
                            "{field}.endPort must be greater than or equal to port"
                        ));
                    }
                }
                if selector.protocol == Some(NetworkProtocol::Icmp)
                    && (selector.port.is_some() || selector.end_port.is_some())
                {
                    return Err(format!("{field} ICMP does not accept port or endPort"));
                }
            }
        }
        Ok(())
    }
}

// MXC 1.1.0 bounds the address expansion of each peer and the full iptables
// rule product. Count the same expansion before a configuration can be saved.
fn validate_bubblewrap_budget(egress: &NetworkEgressConfig) -> Result<(), String> {
    const MAX_RULES: usize = 65_536;
    // Bubblewrap installs the host-loopback gateway deny before caller rules.
    let mut total = 1usize;
    for (kind, rules) in [("deny", &egress.deny), ("allow", &egress.allow)] {
        for (index, rule) in rules.iter().flatten().enumerate() {
            let field = format!("permission.network.{kind}[{index}]");
            let mut addresses = 0usize;
            if let Some(peers) = &rule.to {
                for (index, peer) in peers.iter().enumerate() {
                    addresses = addresses.saturating_add(bubblewrap_peer_blocks(
                        peer,
                        &format!("{field}.to[{index}]"),
                    )?);
                    if addresses > MAX_RULES - total {
                        return Err(bubblewrap_budget_error(&field));
                    }
                }
            } else {
                addresses = 2;
            }
            let ports = rule.ports.as_ref().map_or(1, |ports| {
                ports.iter().fold(0usize, |count, port| {
                    count.saturating_add(
                        if port.protocol.unwrap_or_default() == NetworkProtocol::Any
                            && port.port.is_some()
                        {
                            2
                        } else {
                            1
                        },
                    )
                })
            });
            let expanded = addresses.checked_mul(ports);
            if expanded.is_none_or(|count| count > MAX_RULES - total) {
                return Err(bubblewrap_budget_error(&field));
            }
            total += expanded.expect("bounded expansion");
        }
    }
    Ok(())
}

fn bubblewrap_budget_error(field: &str) -> String {
    format!(
        "{field}: Linux Bubblewrap network policy expands beyond 65536 firewall rules; reduce destinations, exclusions, or port selectors"
    )
}

fn bubblewrap_peer_blocks(peer: &NetworkPeerConfig, field: &str) -> Result<usize, String> {
    let network = bubblewrap_network(&peer.cidr, &format!("{field}.cidr"))?;
    let excepts = peer
        .except
        .iter()
        .enumerate()
        .map(|(index, value)| bubblewrap_network(value, &format!("{field}.except[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut count = 0usize;
    count_bubblewrap_blocks(network, &excepts, &mut count, field)?;
    Ok(count)
}

fn bubblewrap_network(value: &str, field: &str) -> Result<IpNet, String> {
    let network = parse_cidr(value, field)?;
    if let IpNet::V6(network) = network
        && let Some(address) = network.addr().to_ipv4_mapped()
        && network.prefix_len() >= 96
    {
        return Ok(IpNet::new(address.into(), network.prefix_len() - 96)
            .expect("mapped prefix fits IPv4")
            .trunc());
    }
    Ok(network.trunc())
}

fn count_bubblewrap_blocks(
    network: IpNet,
    excepts: &[IpNet],
    count: &mut usize,
    field: &str,
) -> Result<(), String> {
    if excepts.iter().any(|except| except.contains(&network)) {
        return Ok(());
    }
    if !excepts.iter().any(|except| network.contains(except)) {
        *count += 1;
        if *count > 256 {
            return Err(format!(
                "{field}: Linux Bubblewrap permits at most 256 address blocks after except subtraction; narrow the CIDR or reduce exclusions"
            ));
        }
        return Ok(());
    }
    let children = network
        .subnets(network.prefix_len() + 1)
        .map_err(|_| format!("{field}: invalid native CIDR subtraction"))?;
    for child in children {
        count_bubblewrap_blocks(child, excepts, count, field)?;
    }
    Ok(())
}

fn validate_bubblewrap_cidr(value: &str, field: &str) -> Result<(), String> {
    if let IpNet::V6(network) = parse_cidr(value, field)? {
        let mapped_base = std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0, 0);
        if network.prefix_len() < 96 && network.contains(&mapped_base) {
            return Err(format!(
                "{field}: Linux Bubblewrap cannot enforce IPv6 CIDR {value} spanning IPv4-mapped addresses; use explicit IPv4 and native IPv6 subnets, or use to=\"*\" to match both families"
            ));
        }
    }
    Ok(())
}

fn parse_cidr(value: &str, field: &str) -> Result<IpNet, String> {
    if !value.contains('/') || value.trim() != value {
        return Err(format!(
            "{field} must be an explicit IPv4 or IPv6 CIDR; use /32 or /128 for one address"
        ));
    }
    let network = value
        .parse::<IpNet>()
        .map_err(|_| format!("{field} must be a valid IPv4 or IPv6 CIDR"))?;
    if network.addr() != network.network() {
        return Err(format!(
            "{field} must use a network address with zero host bits; use {} instead of {value}",
            network.trunc()
        ));
    }
    Ok(network)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flat_network_round_trips_with_ports_ranges_and_exclusions() {
        let value = json!({
            "default":"deny",
            "allow":[
                {"to":"192.0.2.10","protocol":"tcp","ports":[443,"8000-8099"]},
                {"to":"10.20.0.0/16","except":["10.20.10.0/24"],"protocol":"udp","ports":[53]},
                {"to":"2001:db8::10","protocol":"icmp"}
            ],
            "deny":[{"to":"10.20.66.0/24"},{"to":"*","protocol":"tcp","ports":[25]}]
        });
        let policy: NetworkPermissionConfig = serde_json::from_value(value.clone()).unwrap();
        policy.validate().unwrap();
        assert_eq!(serde_json::to_value(&policy).unwrap(), value);
        let allow = policy.egress.unwrap().allow.unwrap();
        assert_eq!(allow[0].to.as_ref().unwrap()[0].cidr, "192.0.2.10/32");
        assert_eq!(allow[0].ports.as_ref().unwrap()[1].end_port, Some(8099));
        assert_eq!(allow[2].to.as_ref().unwrap()[0].cidr, "2001:db8::10/128");
    }

    #[test]
    fn native_product_keeps_identity_after_flat_transport() {
        use std::hash::{Hash, Hasher};
        let policy = NetworkPermissionConfig::from_legacy_value(json!({
            "egress":{"default":"deny","allow":[{
                "to":[
                    {"cidr":"10.20.0.0/16","except":["10.20.10.0/24"]},
                    {"cidr":"2001:db8::10/128"}
                ],
                "ports":[
                    {"protocol":"tcp","port":443},
                    {"protocol":"tcp","port":8000,"endPort":8099},
                    {"protocol":"udp","port":53}
                ]
            }]}
        }))
        .unwrap();
        let flat = serde_json::to_value(&policy).unwrap();
        assert_eq!(flat["allow"].as_array().unwrap().len(), 4);
        assert_eq!(flat["allow"][0]["ports"], json!([443, "8000-8099"]));
        assert_eq!(flat["allow"][1]["protocol"], "udp");
        assert_eq!(flat["allow"][2]["to"], "2001:db8::10");
        let transported: NetworkPermissionConfig = serde_json::from_value(flat.clone()).unwrap();
        assert_eq!(serde_json::to_value(&transported).unwrap(), flat);
        assert_eq!(transported, policy);
        let hash = |value: &NetworkPermissionConfig| {
            let mut state = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut state);
            state.finish()
        };
        assert_eq!(hash(&transported), hash(&policy));
    }

    #[test]
    fn flat_network_rejects_null_with_editable_field_paths() {
        for (network, field) in [
            (json!(null), "permission.network"),
            (json!({"default":null}), "permission.network.default"),
            (json!({"allow":null}), "permission.network.allow"),
            (json!({"deny":null}), "permission.network.deny"),
            (json!({"inbound":null}), "permission.network.inbound"),
            (
                json!({"hostLoopback":null}),
                "permission.network.hostLoopback",
            ),
            (
                json!({"allow":[{"to":null}]}),
                "permission.network.allow[0].to",
            ),
            (
                json!({"deny":[{"to":"*","ports":null}]}),
                "permission.network.deny[0].ports",
            ),
            (
                json!({"allow":[{"to":"*","protocol":null}]}),
                "permission.network.allow[0].protocol",
            ),
            (
                json!({"allow":[{"to":"10.0.0.0/8","except":null}]}),
                "permission.network.allow[0].except",
            ),
        ] {
            let error =
                serde_json::from_value::<crate::PermissionConfig>(json!({"network":network}))
                    .unwrap_err()
                    .to_string();
            assert!(error.contains(&format!("{field}:")), "{error}");
            assert!(error.contains("null"), "{error}");
        }
    }

    #[test]
    fn omitted_network_inherits_but_present_blocks_replace_fully() {
        let original: crate::PermissionConfig = serde_json::from_value(json!({
            "network":{
                "default":"allow",
                "allow":[{"to":"192.0.2.0/24"}],
                "deny":[{"to":"192.0.2.128/25"}]
            }
        }))
        .unwrap();
        let mut inherited = original.clone();
        inherited.merge_from(serde_json::from_value(json!({})).unwrap());
        assert_eq!(inherited, original);
        for network in [json!({}), json!({"default":"deny"}), json!({"allow":[]})] {
            let overlay: crate::PermissionConfig =
                serde_json::from_value(json!({"network":network})).unwrap();
            assert!(!overlay.is_empty());
            let mut merged = original.clone();
            merged.merge_from(overlay);
            assert_eq!(merged.network.unwrap(), NetworkPermissionConfig::offline());
        }
        let mut replaced = original;
        replaced.merge_from(
            serde_json::from_value(json!({"network":{"allow":[{"to":"198.51.100.7"}]}})).unwrap(),
        );
        assert_eq!(
            serde_json::to_value(replaced).unwrap(),
            json!({
                "network":{"default":"deny","allow":[{"to":"198.51.100.7"}]}
            })
        );
    }

    #[test]
    fn explicit_wildcard_and_omitted_selectors_match_native_any() {
        let config: NetworkPermissionConfig =
            serde_json::from_value(json!({"allow":[{"to":"*"}]})).unwrap();
        let rules = config.egress.unwrap().allow.unwrap();
        assert!(rules[0].to.is_none());
        assert!(rules[0].ports.is_none());
        let empty_except: NetworkPermissionConfig = serde_json::from_value(json!({
            "allow":[{"to":"*","except":[]}]
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(empty_except).unwrap(),
            json!({
                "default":"deny","allow":[{"to":"*"}]
            })
        );
        assert!(serde_json::from_value::<NetworkPermissionConfig>(json!({"allow":[{}]})).is_err());
    }

    #[test]
    fn flat_network_rejects_invalid_rules_with_canonical_cidr_suggestions() {
        for (rule, field, canonical) in [
            (
                json!({"to":"192.0.2.7/24"}),
                "permission.network.allow[0].to",
                "192.0.2.0/24",
            ),
            (
                json!({"to":"2001:db8::7/64"}),
                "permission.network.allow[0].to",
                "2001:db8::/64",
            ),
            (
                json!({"to":"192.0.2.0/24","except":["192.0.2.129/25"]}),
                "permission.network.allow[0].except[0]",
                "192.0.2.128/25",
            ),
        ] {
            let error = serde_json::from_value::<NetworkPermissionConfig>(json!({"allow":[rule]}))
                .unwrap_err()
                .to_string();
            assert!(error.contains(field), "{error}");
            assert!(
                error.contains(&format!("use {canonical} instead")),
                "{error}"
            );
        }
        for rule in [
            json!({"to":"example.com"}),
            json!({"to":"https://192.0.2.1"}),
            json!({"to":"*","except":["192.0.2.0/24"]}),
            json!({"to":"10.0.0.0/8","except":["192.0.2.0/24"]}),
            json!({"to":"*","ports":[]}),
            json!({"to":"*","ports":[0]}),
            json!({"to":"*","ports":[65536]}),
            json!({"to":"*","ports":[443.5]}),
            json!({"to":"*","ports":["443"]}),
            json!({"to":"*","ports":["0-80"]}),
            json!({"to":"*","ports":["8000-7000"]}),
            json!({"to":"*","ports":["+80-90"]}),
            json!({"to":"*","ports":["080-90"]}),
            json!({"to":"*","protocol":"icmp","ports":[53]}),
        ] {
            assert!(
                serde_json::from_value::<NetworkPermissionConfig>(json!({"allow":[rule.clone()]}))
                    .is_err(),
                "{rule}"
            );
        }
    }

    #[test]
    fn legacy_layers_are_resolved_explicitly_before_flat_migration() {
        let mut policy = NetworkPermissionConfig::from_legacy_value(json!({
            "egress":{"default":"allow","deny":[{"to":[{"cidr":"10.0.0.0/8"}]}]}
        }))
        .unwrap();
        let overlay = NetworkPermissionConfig::from_legacy_value(json!({
            "egress":{"allow":[{"to":[{"cidr":"192.0.2.10/32"}]}]}
        }))
        .unwrap();
        policy.merge_legacy_from(overlay);
        assert_eq!(
            serde_json::to_value(&policy).unwrap(),
            json!({
                "default":"allow","allow":[{"to":"192.0.2.10"}],"deny":[{"to":"10.0.0.0/8"}]
            })
        );
    }

    #[test]
    fn obsolete_network_schema_and_approval_modes_are_rejected() {
        for value in [
            json!({"internet":"allow"}),
            json!({"private":"deny"}),
            json!({"loopback":"deny"}),
            json!({"rules":{}}),
            json!({"egress":{}}),
            json!({"ingress":{}}),
            json!({"default":"ask"}),
            json!({"default":"auto"}),
            json!({"allow":[{"to":"*","domain":"example.com"}]}),
            json!({"host_loopback":"deny"}),
        ] {
            assert!(
                serde_json::from_value::<NetworkPermissionConfig>(value.clone()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn native_validation_rejects_unenforceable_selectors() {
        for rule in [
            json!({"to":[]}),
            json!({"ports":[]}),
            json!({"to":[{"cidr":"example.com"}]}),
            json!({"to":[{"cidr":"192.0.2.1"}]}),
            json!({"to":[{"cidr":"192.0.2.0/33"}]}),
            json!({"to":[{"cidr":"192.0.2.0/24","except":["198.51.100.0/24"]}]}),
            json!({"to":[{"cidr":"192.0.2.0/24","except":["::/0"]}]}),
            json!({"ports":[{"protocol":"tcp","port":0}]}),
            json!({"ports":[{"protocol":"tcp","endPort":443}]}),
            json!({"ports":[{"protocol":"udp","port":9000,"endPort":8000}]}),
            json!({"ports":[{"protocol":"icmp","port":53}]}),
        ] {
            let policy: NetworkPermissionConfig =
                NetworkPermissionConfig::from_legacy_value(json!({
                    "egress":{"allow":[rule.clone()]}
                }))
                .unwrap();
            assert!(policy.validate().is_err(), "{rule}");
        }
    }

    #[test]
    fn omitted_effective_defaults_are_offline() {
        assert_eq!(
            NetworkPermissionConfig::default().resolved(),
            NetworkPermissionConfig::offline()
        );
        let policy = NetworkPermissionConfig::allow_all();
        assert_eq!(policy.egress.unwrap().default, Some(NetworkAction::Allow));
        let ingress = policy.ingress.unwrap();
        assert_eq!(ingress.default, Some(NetworkAction::Deny));
        assert_eq!(ingress.host_loopback, Some(NetworkAction::Deny));
    }

    #[test]
    fn linux_peer_expansion_checks_the_exact_native_block_budget() {
        let base = u32::from(std::net::Ipv4Addr::new(10, 0, 0, 0));
        for (prefix, size, accepted) in [(23, 512u32, true), (22, 1024u32, false)] {
            let except = (1..size)
                .step_by(2)
                .map(|offset| format!("{}/32", std::net::Ipv4Addr::from(base + offset)))
                .collect::<Vec<_>>();
            let policy: NetworkPermissionConfig = NetworkPermissionConfig::from_legacy_value(json!({
                "egress":{"allow":[{"to":[{"cidr":format!("10.0.0.0/{prefix}"),"except":except}]}]}
            }))
            .unwrap();
            let result = policy.validate_for_platform("linux");
            assert_eq!(result.is_ok(), accepted, "{result:?}");
            if let Err(error) = result {
                assert!(error.contains("256 address blocks"), "{error}");
            }
        }
    }

    #[test]
    fn linux_rule_budget_counts_protocol_expansion_and_the_gateway_guard() {
        let mut ports = vec![
            NetworkPortConfig {
                protocol: Some(NetworkProtocol::Any),
                port: Some(443),
                end_port: None,
            };
            32_767
        ];
        ports.push(NetworkPortConfig {
            protocol: Some(NetworkProtocol::Tcp),
            port: Some(443),
            end_port: None,
        });
        let mut policy = NetworkPermissionConfig::offline();
        policy.egress.as_mut().unwrap().allow = Some(vec![NetworkRuleConfig {
            to: Some(vec![NetworkPeerConfig {
                cidr: "192.0.2.7/32".into(),
                except: vec![],
            }]),
            ports: Some(ports),
        }]);
        policy.validate_for_platform("linux").unwrap();
        policy.egress.as_mut().unwrap().allow.as_mut().unwrap()[0]
            .ports
            .as_mut()
            .unwrap()
            .push(NetworkPortConfig {
                protocol: Some(NetworkProtocol::Tcp),
                port: Some(80),
                end_port: None,
            });
        assert!(
            policy
                .validate_for_platform("linux")
                .unwrap_err()
                .contains("65536 firewall rules")
        );
    }

    #[test]
    fn linux_omitted_destinations_count_both_ip_families() {
        let mut policy = NetworkPermissionConfig::offline();
        policy.egress.as_mut().unwrap().allow = Some(vec![NetworkRuleConfig {
            to: None,
            ports: Some(vec![
                NetworkPortConfig {
                    protocol: Some(NetworkProtocol::Any),
                    port: Some(443),
                    end_port: None
                };
                16_384
            ]),
        }]);
        assert!(
            policy
                .validate_for_platform("linux")
                .unwrap_err()
                .contains("65536 firewall rules")
        );
    }

    #[test]
    fn linux_rejects_ipv6_ranges_straddling_mapped_addresses() {
        for cidr in ["::/0", "::/80", "::fffe:0:0/95"] {
            let policy: NetworkPermissionConfig =
                NetworkPermissionConfig::from_legacy_value(json!({
                    "egress":{"allow":[{"to":[{"cidr":cidr}]}]}
                }))
                .unwrap();
            assert!(policy.validate().is_ok());
            assert!(
                policy
                    .validate_for_platform("linux")
                    .unwrap_err()
                    .contains("IPv4-mapped"),
                "{cidr}"
            );
        }
        for cidr in ["2001:db8::/32", "fd00::/8", "::ffff:192.0.2.0/120"] {
            let policy: NetworkPermissionConfig =
                NetworkPermissionConfig::from_legacy_value(json!({
                    "egress":{"allow":[{"to":[{"cidr":cidr}]}]}
                }))
                .unwrap();
            policy.validate_for_platform("linux").unwrap();
        }
        let all: NetworkPermissionConfig = NetworkPermissionConfig::from_legacy_value(json!({
            "egress":{"allow":[{}]}
        }))
        .unwrap();
        all.validate_for_platform("linux").unwrap();
    }

    #[test]
    fn platform_validation_matches_native_backend_capabilities() {
        let fine_grained: NetworkPermissionConfig =
            NetworkPermissionConfig::from_legacy_value(json!({
                "egress":{"allow":[{"to":[{"cidr":"192.0.2.1/32"}]}]}
            }))
            .unwrap();
        assert!(fine_grained.validate_for_platform("linux").is_ok());
        assert!(fine_grained.validate_for_platform("windows").is_ok());
        assert!(
            fine_grained
                .validate_for_platform("macos")
                .unwrap_err()
                .contains("network.allow")
        );
        for value in [
            json!({"ingress":{"default":"allow"}}),
            json!({"ingress":{"hostLoopback":"allow"}}),
        ] {
            let policy: NetworkPermissionConfig =
                NetworkPermissionConfig::from_legacy_value(value).unwrap();
            assert!(policy.validate_for_platform("linux").is_err());
            assert!(policy.validate_for_platform("windows").is_ok());
        }
        let loopback: NetworkPermissionConfig = NetworkPermissionConfig::from_legacy_value(json!({
            "ingress":{"hostLoopback":"allow"}
        }))
        .unwrap();
        assert!(loopback.validate_for_platform("macos").is_err());
        let bidirectional: NetworkPermissionConfig =
            NetworkPermissionConfig::from_legacy_value(json!({
                "ingress":{"default":"allow","hostLoopback":"allow"}
            }))
            .unwrap();
        assert!(bidirectional.validate_for_platform("macos").is_ok());
        for platform in ["linux", "macos", "windows"] {
            NetworkPermissionConfig::offline()
                .validate_for_platform(platform)
                .unwrap();
            NetworkPermissionConfig::allow_all()
                .validate_for_platform(platform)
                .unwrap();
        }
    }
}
