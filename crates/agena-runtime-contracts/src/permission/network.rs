//! Native MXC network semantics shared by preflight and subprocess policies.
use super::{PermissionConfigError, PermissionDecision};
use agena_domain::{
    NetworkAction, NetworkEgressConfig, NetworkIngressConfig, NetworkPeerConfig,
    NetworkPermissionConfig, NetworkPortConfig, NetworkProtocol, NetworkRuleConfig, NetworkTarget,
};
use ipnet::IpNet;
use std::collections::HashSet;
use std::net::IpAddr;

// Bound newly expanded intersections at the native Bubblewrap rule budget.
// Direct input and accumulated denies are checked by the final platform
// validator; they must not acquire a smaller application-only rule limit.
const MAX_INTERSECTION_RULES: usize = 65_536;

#[derive(Debug, Clone)]
pub struct NetworkPermissionPolicy {
    pub(crate) config: NetworkPermissionConfig,
}

impl NetworkPermissionPolicy {
    pub fn new(default: NetworkAction) -> Self {
        let mut config = NetworkPermissionConfig::offline();
        config.egress.as_mut().expect("resolved egress").default = Some(default);
        Self { config }
    }

    pub fn allow_all() -> Self {
        Self::new(NetworkAction::Allow)
    }

    pub fn from_config(config: &NetworkPermissionConfig) -> Result<Self, PermissionConfigError> {
        let config = config.simplified_for_sandbox().map_err(|reason| {
            PermissionConfigError::InvalidNetworkRule {
                pattern: "permission.network".into(),
                reason,
            }
        })?;
        Ok(Self { config })
    }

    /// Hostnames must be resolved by the caller; a DNS name is never a grant.
    /// "*" is the explicit request for the configured native policy as a whole.
    pub fn check_connect(&self, target: &NetworkTarget) -> PermissionDecision {
        if target.host() == "*" {
            let egress = self.config.egress.as_ref().expect("resolved egress");
            let ingress = self.config.ingress.as_ref().expect("resolved ingress");
            // A full native scope can grant only inbound or host-loopback
            // access. Platform validation still rejects unsupported flags.
            return decision(
                egress.default == Some(NetworkAction::Allow)
                    || egress.allow.as_ref().is_some_and(|rules| !rules.is_empty())
                    || ingress.default == Some(NetworkAction::Allow)
                    || ingress.host_loopback == Some(NetworkAction::Allow),
                "the native network policy grants no network access",
            );
        }
        let Ok(address) = target.host().parse::<IpAddr>() else {
            return denied("native network checks require a resolved IP address");
        };
        let protocol = target_protocol(target);
        match target.port() {
            Some(port) => self.check_endpoint(address, protocol, Some(port)),
            None => {
                // Test selector boundaries, not every possible port. Decisions are
                // constant between these boundaries for this fixed destination.
                let mut ports = vec![1u16, 65535];
                let egress = self.config.egress.as_ref().expect("resolved egress");
                for rule in egress.allow.iter().chain(egress.deny.iter()).flatten() {
                    for selector in rule.ports.iter().flatten() {
                        if let Some(port) = selector.port {
                            ports.push(port);
                            if port > 1 {
                                ports.push(port - 1);
                            }
                            let end = selector.end_port.unwrap_or(port);
                            ports.push(end);
                            if end < u16::MAX {
                                ports.push(end + 1);
                            }
                        }
                    }
                }
                let protocols = match protocol {
                    NetworkProtocol::Any => vec![
                        NetworkProtocol::Tcp,
                        NetworkProtocol::Udp,
                        NetworkProtocol::Icmp,
                        NetworkProtocol::Any,
                    ],
                    protocol => vec![protocol],
                };
                decision(
                    protocols.into_iter().any(|protocol| {
                        if matches!(protocol, NetworkProtocol::Icmp | NetworkProtocol::Any) {
                            matches!(
                                self.check_endpoint(address, protocol, None),
                                PermissionDecision::Allow
                            )
                        } else {
                            ports.iter().any(|port| {
                                matches!(
                                    self.check_endpoint(address, protocol, Some(*port)),
                                    PermissionDecision::Allow
                                )
                            })
                        }
                    }),
                    "the native network policy denies this destination",
                )
            }
        }
    }

    pub fn check_endpoint(
        &self,
        address: IpAddr,
        protocol: NetworkProtocol,
        port: Option<u16>,
    ) -> PermissionDecision {
        let address = canonical_ip(address);
        let ingress = self.config.ingress.as_ref().expect("resolved ingress");
        if address.is_loopback() && ingress.host_loopback != Some(NetworkAction::Allow) {
            return denied("native host-loopback access is denied");
        }
        let egress = self.config.egress.as_ref().expect("resolved egress");
        if egress.deny.as_ref().is_some_and(|rules| {
            rules
                .iter()
                .any(|rule| rule_matches(rule, address, protocol, port))
        }) {
            return denied("matched a native MXC network deny rule");
        }
        decision(
            egress.default == Some(NetworkAction::Allow)
                || egress.allow.as_ref().is_some_and(|rules| {
                    rules
                        .iter()
                        .any(|rule| rule_matches(rule, address, protocol, port))
                }),
            "no native MXC network allow rule matches this destination",
        )
    }
}

pub(crate) fn target_protocol(target: &NetworkTarget) -> NetworkProtocol {
    match target
        .original()
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
    {
        Some(scheme) if scheme == "udp" => NetworkProtocol::Udp,
        Some(scheme) if scheme == "icmp" => NetworkProtocol::Icmp,
        Some(_) => NetworkProtocol::Tcp,
        None if target.port().is_some() => NetworkProtocol::Tcp,
        None => NetworkProtocol::Any,
    }
}

pub(crate) fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        ip => ip,
    }
}

fn decision(allowed: bool, reason: &str) -> PermissionDecision {
    if allowed {
        PermissionDecision::Allow
    } else {
        denied(reason)
    }
}

fn denied(reason: &str) -> PermissionDecision {
    PermissionDecision::Deny {
        reason: reason.into(),
    }
}

fn rule_matches(
    rule: &NetworkRuleConfig,
    address: IpAddr,
    protocol: NetworkProtocol,
    port: Option<u16>,
) -> bool {
    rule.to.as_ref().is_none_or(|peers| {
        peers.iter().any(|peer| {
            parse_network(&peer.cidr).is_ok_and(|net| net.contains(&address))
                && !peer
                    .except
                    .iter()
                    .any(|except| parse_network(except).is_ok_and(|net| net.contains(&address)))
        })
    }) && rule.ports.as_ref().is_none_or(|ports| {
        ports.iter().any(|selector| {
            let wanted = selector.protocol.unwrap_or_default();
            if wanted != NetworkProtocol::Any && wanted != protocol {
                return false;
            }
            match selector.port {
                None => true,
                Some(start) => {
                    matches!(protocol, NetworkProtocol::Tcp | NetworkProtocol::Udp)
                        && port.is_some_and(|port| {
                            (start..=selector.end_port.unwrap_or(start)).contains(&port)
                        })
                }
            }
        })
    })
}

/// Intersection of independent authority layers. Effective denies are retained;
/// only default-deny layers constrain the union of positive grants.
pub(crate) fn intersect_policies(
    layers: &[NetworkPermissionConfig],
) -> Result<NetworkPermissionConfig, String> {
    let mut positive: Option<Vec<NetworkRuleConfig>> = None;
    let mut applied_positive_sets = Vec::new();
    let mut deny = Vec::new();
    let mut seen_denies = HashSet::new();
    let mut inbound = NetworkAction::Allow;
    let mut loopback = NetworkAction::Allow;
    for layer in layers {
        let layer = layer.simplified_for_sandbox()?;
        let egress = layer.egress.expect("resolved egress");
        let ingress = layer.ingress.expect("resolved ingress");
        if ingress.default == Some(NetworkAction::Deny) {
            inbound = NetworkAction::Deny;
        }
        if ingress.host_loopback == Some(NetworkAction::Deny) {
            loopback = NetworkAction::Deny;
        }
        for rule in egress.deny.unwrap_or_default() {
            if seen_denies.insert(rule.clone()) {
                deny.push(rule);
            }
        }
        if egress.default == Some(NetworkAction::Deny) {
            let mut allow = egress.allow.unwrap_or_default();
            dedup(&mut allow);
            let allow_set = allow.iter().cloned().collect::<HashSet<_>>();
            if applied_positive_sets.contains(&allow_set) {
                continue;
            }
            applied_positive_sets.push(allow_set);
            positive = Some(match positive {
                None => allow,
                Some(previous) => {
                    let mut result = Vec::new();
                    let mut seen = HashSet::new();
                    for left in &previous {
                        for right in &allow {
                            if let Some(rule) = intersect_rules(left, right)? {
                                if seen.insert(rule.clone()) {
                                    result.push(rule);
                                }
                                if result.len() > MAX_INTERSECTION_RULES {
                                    return Err("native network policy intersection expands beyond 65536 rules; simplify the overlapping authority layers".into());
                                }
                            }
                        }
                    }
                    result
                }
            });
        }
    }
    let default = if positive.is_some() {
        NetworkAction::Deny
    } else {
        NetworkAction::Allow
    };
    let result = NetworkPermissionConfig {
        egress: Some(NetworkEgressConfig {
            default: Some(default),
            allow: Some(positive.unwrap_or_default()),
            deny: Some(deny),
        }),
        ingress: Some(NetworkIngressConfig {
            default: Some(inbound),
            host_loopback: Some(loopback),
        }),
    };
    result.simplified_for_sandbox()
}

fn dedup<T: Clone + Eq + std::hash::Hash>(values: &mut Vec<T>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn intersect_rules(
    left: &NetworkRuleConfig,
    right: &NetworkRuleConfig,
) -> Result<Option<NetworkRuleConfig>, String> {
    if left == right {
        return Ok(Some(left.clone()));
    }
    // Omission is MXC's dual-family wildcard. Explicit ::/0 is not equivalent:
    // Bubblewrap rejects IPv6 blocks straddling the IPv4-mapped address range.
    let to = match (&left.to, &right.to) {
        (None, None) => None,
        (Some(peers), None) | (None, Some(peers)) => Some(peers.clone()),
        (Some(left), Some(right)) => {
            let mut peers = Vec::new();
            for left in left {
                for right in right {
                    if let Some(peer) = intersect_peers(left, right)? {
                        peers.push(peer);
                    }
                }
            }
            if peers.is_empty() {
                return Ok(None);
            }
            dedup(&mut peers);
            Some(peers)
        }
    };
    let all_ports = vec![NetworkPortConfig::default()];
    let mut ports = Vec::new();
    for left in left.ports.as_ref().unwrap_or(&all_ports) {
        for right in right.ports.as_ref().unwrap_or(&all_ports) {
            if let Some(selector) = intersect_ports(left, right) {
                ports.push(selector);
            }
        }
    }
    if ports.is_empty() {
        return Ok(None);
    }
    dedup(&mut ports);
    Ok(Some(NetworkRuleConfig {
        to,
        ports: Some(ports),
    }))
}

/// MXC programs mapped IPv6 CIDRs as IPv4 packet-filter entries.
fn parse_network(value: &str) -> Result<IpNet, String> {
    let network = value.parse::<IpNet>().map_err(|error| error.to_string())?;
    match network {
        IpNet::V6(network) if network.prefix_len() >= 96 => {
            if let Some(address) = network.addr().to_ipv4_mapped() {
                return ipnet::Ipv4Net::new(address, network.prefix_len() - 96)
                    .map(|network| IpNet::V4(network.trunc()))
                    .map_err(|error| error.to_string());
            }
            Ok(IpNet::V6(network.trunc()))
        }
        network => Ok(network.trunc()),
    }
}

fn intersection(left: IpNet, right: IpNet) -> Option<IpNet> {
    if left.contains(&right.network()) && left.prefix_len() <= right.prefix_len() {
        Some(right.trunc())
    } else if right.contains(&left.network()) && right.prefix_len() <= left.prefix_len() {
        Some(left.trunc())
    } else {
        None
    }
}

fn intersect_peers(
    left: &NetworkPeerConfig,
    right: &NetworkPeerConfig,
) -> Result<Option<NetworkPeerConfig>, String> {
    let Some(base) = intersection(parse_network(&left.cidr)?, parse_network(&right.cidr)?) else {
        return Ok(None);
    };
    let mut except = Vec::new();
    for excluded in left.except.iter().chain(&right.except) {
        if let Some(excluded) = intersection(base, parse_network(excluded)?) {
            if excluded == base {
                return Ok(None);
            }
            except.push(excluded.to_string());
        }
    }
    except.sort();
    except.dedup();
    Ok(Some(NetworkPeerConfig {
        cidr: base.to_string(),
        except,
    }))
}

fn intersect_ports(
    left: &NetworkPortConfig,
    right: &NetworkPortConfig,
) -> Option<NetworkPortConfig> {
    let left_protocol = left.protocol.unwrap_or_default();
    let right_protocol = right.protocol.unwrap_or_default();
    let protocol = match (left_protocol, right_protocol) {
        (NetworkProtocol::Any, other) | (other, NetworkProtocol::Any) => other,
        (left, right) if left == right => left,
        _ => return None,
    };
    if protocol == NetworkProtocol::Icmp && (left.port.is_some() || right.port.is_some()) {
        return None;
    }
    let start = left.port.unwrap_or(1).max(right.port.unwrap_or(1));
    let end = left
        .port
        .map(|port| left.end_port.unwrap_or(port))
        .unwrap_or(u16::MAX)
        .min(
            right
                .port
                .map(|port| right.end_port.unwrap_or(port))
                .unwrap_or(u16::MAX),
        );
    if start > end {
        return None;
    }
    let has_port = left.port.is_some() || right.port.is_some();
    Some(NetworkPortConfig {
        protocol: Some(protocol),
        port: has_port.then_some(start),
        end_port: (has_port && end != start).then_some(end),
    })
}

#[cfg(test)]
mod tests;
