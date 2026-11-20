//! Human-editable network settings. Native policy composition stays in the parent.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use super::{
    NetworkAction, NetworkEgressConfig, NetworkIngressConfig, NetworkPeerConfig,
    NetworkPermissionConfig, NetworkPortConfig, NetworkProtocol, NetworkRuleConfig,
    deserialize_present, parse_cidr,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NetworkDocument {
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    version: Option<u8>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    default: Option<NetworkAction>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    allow: Option<Vec<RuleDocument>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    deny: Option<Vec<RuleDocument>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    inbound: Option<NetworkAction>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    host_loopback: Option<NetworkAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleDocument {
    to: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    except: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "is_any")]
    protocol: NetworkProtocol,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present"
    )]
    ports: Option<Vec<PortDocument>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
enum PortDocument {
    Port(u16),
    Range(String),
}

fn is_any(value: &NetworkProtocol) -> bool {
    *value == NetworkProtocol::Any
}

impl NetworkDocument {
    pub(super) fn into_native(self) -> Result<NetworkPermissionConfig, String> {
        // Unversioned flat policies historically replaced the whole block.
        // Materialize their omitted denies/empty lists so upgrading never
        // reopens a permission through a newly inherited parent value.
        let legacy = match self.version {
            None => true,
            Some(2) => false,
            Some(version) => {
                return Err(format!(
                    "permission.network.version {version} is unsupported; expected 2"
                ));
            }
        };
        let rules = |rules: Vec<RuleDocument>, kind| {
            rules
                .into_iter()
                .enumerate()
                .map(|(index, rule)| {
                    rule.into_native(&format!("permission.network.{kind}[{index}]"))
                })
                .collect::<Result<Vec<_>, String>>()
        };
        let egress = NetworkEgressConfig {
            default: self
                .default
                .or_else(|| legacy.then_some(NetworkAction::Deny)),
            allow: self
                .allow
                .map(|value| rules(value, "allow"))
                .transpose()?
                .or_else(|| legacy.then(Vec::new)),
            deny: self
                .deny
                .map(|value| rules(value, "deny"))
                .transpose()?
                .or_else(|| legacy.then(Vec::new)),
        };
        let ingress = NetworkIngressConfig {
            default: self
                .inbound
                .or_else(|| legacy.then_some(NetworkAction::Deny)),
            host_loopback: self
                .host_loopback
                .or_else(|| legacy.then_some(NetworkAction::Deny)),
        };
        Ok(NetworkPermissionConfig {
            egress: (!egress.is_empty()).then_some(egress),
            ingress: (!ingress.is_empty()).then_some(ingress),
        })
    }

    pub(super) fn from_native(config: &NetworkPermissionConfig) -> Result<Self, String> {
        config.validate()?;
        let egress = config.egress.as_ref();
        let ingress = config.ingress.as_ref();
        let flatten = |rules: &Vec<NetworkRuleConfig>| {
            let mut result = Vec::new();
            for rule in rules {
                result.extend(RuleDocument::from_native(rule)?);
            }
            Ok::<_, String>(result)
        };
        Ok(Self {
            version: Some(2),
            default: egress.and_then(|value| value.default),
            allow: egress
                .and_then(|value| value.allow.as_ref())
                .map(flatten)
                .transpose()?,
            deny: egress
                .and_then(|value| value.deny.as_ref())
                .map(flatten)
                .transpose()?,
            inbound: ingress.and_then(|value| value.default),
            host_loopback: ingress.and_then(|value| value.host_loopback),
        })
    }
}

impl RuleDocument {
    fn into_native(self, field: &str) -> Result<NetworkRuleConfig, String> {
        let to = if self.to == "*" {
            if self
                .except
                .as_ref()
                .is_some_and(|values| !values.is_empty())
            {
                return Err(format!(
                    "{field}.except requires a specific CIDR target; '*' has no subnet exclusions"
                ));
            }
            None
        } else {
            let cidr = if self.to.contains('/') {
                parse_cidr(&self.to, &format!("{field}.to"))?
            } else {
                let address = self.to.parse::<IpAddr>().map_err(|_| {
                    format!(
                        "{field}.to must be an IP address, a CIDR, or '*'; domains and URLs are not supported"
                    )
                })?;
                ipnet::IpNet::new(address, if address.is_ipv4() { 32 } else { 128 })
                    .expect("host prefix fits its address family")
            };
            let authored_except = self.except.unwrap_or_default();
            let mut except = Vec::with_capacity(authored_except.len());
            for (index, excluded) in authored_except.into_iter().enumerate() {
                let at = format!("{field}.except[{index}]");
                let excluded = parse_cidr(&excluded, &at)?;
                if !cidr.contains(&excluded) {
                    return Err(format!("{at} must be a subnet within {cidr}"));
                }
                except.push(excluded.to_string());
            }
            Some(vec![NetworkPeerConfig {
                cidr: cidr.to_string(),
                except,
            }])
        };

        let ports = match self.ports {
            Some(ports) => {
                if ports.is_empty() {
                    return Err(format!(
                        "{field}.ports must not be empty; omit it to match all ports"
                    ));
                }
                if self.protocol == NetworkProtocol::Icmp {
                    return Err(format!("{field}.ports: ICMP does not accept ports"));
                }
                Some(
                    ports
                        .into_iter()
                        .enumerate()
                        .map(|(index, port)| {
                            port.into_native(self.protocol, &format!("{field}.ports[{index}]"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            None if self.protocol == NetworkProtocol::Any => None,
            None => Some(vec![NetworkPortConfig {
                protocol: Some(self.protocol),
                port: None,
                end_port: None,
            }]),
        };
        Ok(NetworkRuleConfig { to, ports })
    }

    fn from_native(rule: &NetworkRuleConfig) -> Result<Vec<Self>, String> {
        // A native rule matches the product of its peers and selectors. Split
        // this product into rows without changing protocol/port associations.
        let mut groups: Vec<(NetworkProtocol, Option<Vec<PortDocument>>)> = Vec::new();
        if let Some(selectors) = &rule.ports {
            for selector in selectors {
                let protocol = selector.protocol.unwrap_or_default();
                let port = selector.port.map(|start| match selector.end_port {
                    Some(end) if end != start => PortDocument::Range(format!("{start}-{end}")),
                    _ => PortDocument::Port(start),
                });
                if let Some((_, ports)) = groups.iter_mut().find(|(value, _)| *value == protocol) {
                    match (ports.as_mut(), port) {
                        (Some(ports), Some(port)) => ports.push(port),
                        (_, None) => *ports = None,
                        (None, Some(_)) => {}
                    }
                } else {
                    groups.push((protocol, port.map(|port| vec![port])));
                }
            }
        } else {
            groups.push((NetworkProtocol::Any, None));
        }

        let peers: Vec<_> = match &rule.to {
            Some(peers) => peers.iter().map(Some).collect(),
            None => vec![None],
        };
        let mut result = Vec::new();
        for peer in peers {
            let (to, except) = match peer {
                Some(peer) => {
                    let cidr = parse_cidr(&peer.cidr, "permission.network.to")?;
                    let host_prefix = if cidr.addr().is_ipv4() { 32 } else { 128 };
                    let to = if cidr.prefix_len() == host_prefix {
                        cidr.addr().to_string()
                    } else {
                        cidr.to_string()
                    };
                    (to, peer.except.clone())
                }
                None => ("*".to_owned(), Vec::new()),
            };
            for (protocol, ports) in &groups {
                result.push(Self {
                    to: to.clone(),
                    except: (!except.is_empty()).then(|| except.clone()),
                    protocol: *protocol,
                    ports: ports.clone(),
                });
            }
        }
        Ok(result)
    }
}

impl PortDocument {
    fn into_native(
        self,
        protocol: NetworkProtocol,
        field: &str,
    ) -> Result<NetworkPortConfig, String> {
        let (start, end) = match self {
            Self::Port(port) => (port, None),
            Self::Range(value) => {
                let range = value.split_once('-').and_then(|(start, end)| {
                    let port = |text: &str| {
                        (!text.is_empty()
                            && !text.starts_with('0')
                            && text.bytes().all(|byte| byte.is_ascii_digit()))
                        .then(|| text.parse::<u16>().ok())
                        .flatten()
                    };
                    Some((port(start)?, port(end)?))
                });
                let Some((start, end)) = range else {
                    return Err(format!(
                        "{field} must be an integer port or a range such as '8000-8099'"
                    ));
                };
                if end < start {
                    return Err(format!(
                        "{field} range end must not be smaller than its start"
                    ));
                }
                (start, Some(end))
            }
        };
        if start == 0 || end == Some(0) {
            return Err(format!("{field} ports must be between 1 and 65535"));
        }
        Ok(NetworkPortConfig {
            protocol: Some(protocol),
            port: Some(start),
            end_port: end.filter(|end| *end != start),
        })
    }
}

// The retired nested shape is read only by the explicit migration boundary.
// Keeping it separate prevents ordinary settings/API writes from accepting it.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LegacyNetworkDocument {
    #[serde(default, deserialize_with = "deserialize_present")]
    egress: Option<NetworkEgressConfig>,
    #[serde(default, deserialize_with = "deserialize_present")]
    ingress: Option<NetworkIngressConfig>,
}

impl LegacyNetworkDocument {
    pub(super) fn into_native(self) -> NetworkPermissionConfig {
        NetworkPermissionConfig {
            egress: self.egress,
            ingress: self.ingress,
        }
    }
}
