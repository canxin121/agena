//! A Fake-IP answer is a routing token owned by a trusted local TUN proxy,
//! not the destination's network address. Only the server operator can opt
//! exact synthetic subnets into this exception; tool inputs cannot enable it.
use super::*;
use ipnet::IpNet;
use std::net::SocketAddr;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WebNetworkConfig {
    /// None uses standard environment/OS proxies. Empty forces direct access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Synthetic DNS ranges managed by a trusted, active local TUN proxy.
    /// Empty preserves strict public-address validation.
    pub fake_ip_ranges: Vec<String>,
}

impl std::fmt::Debug for WebNetworkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebNetworkConfig")
            .field("proxy", &self.proxy.as_ref().map(|_| "<configured>"))
            .field("fake_ip_ranges", &self.fake_ip_ranges)
            .finish()
    }
}

fn ranges(config: &WebNetworkConfig) -> SdkResult<Vec<IpNet>> {
    if config.fake_ip_ranges.len() > 16 {
        return Err(PluginError::invalid_params(
            "network.fake_ip_ranges accepts at most 16 synthetic CIDRs",
        ));
    }
    config.fake_ip_ranges.iter().map(|value| {
        let range = value.parse::<IpNet>().map_err(|_| PluginError::invalid_params(
            "network.fake_ip_ranges entries must be canonical IPv4 or IPv6 CIDRs",
        ))?;
        let valid = match range {
            IpNet::V4(net) => net.prefix_len() >= 15
                && net.addr().octets()[0] == 198
                && matches!(net.addr().octets()[1], 18 | 19),
            IpNet::V6(net) => net.prefix_len() >= 64 && net.addr().is_unique_local(),
        };
        if !valid || range.addr() != range.network() {
            return Err(PluginError::invalid_params(
                "network.fake_ip_ranges requires canonical subnets of 198.18.0.0/15 or IPv6 ULA /64 or narrower; configure only the exact ranges owned by your local TUN proxy",
            ));
        }
        Ok(range)
    }).collect()
}

pub(super) fn validate_config(config: &WebNetworkConfig) -> SdkResult<()> {
    if let Some(proxy) = config.proxy.as_deref().filter(|s| !s.is_empty()) {
        let proxy = if proxy.contains("://") {
            proxy.to_owned()
        } else {
            format!("http://{proxy}")
        };
        let url = url::Url::parse(&proxy).map_err(|_| {
            PluginError::invalid_params("network.proxy must be an HTTP(S) or SOCKS proxy URL")
        })?;
        if url.host_str().is_none()
            || !matches!(
                url.scheme(),
                "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
            )
        {
            return Err(PluginError::invalid_params(
                "network.proxy must be an HTTP(S) or SOCKS proxy URL",
            ));
        }
    }
    ranges(config).map(|_| ())
}

/// With remote DNS, local DNS results are neither used nor authoritative.
/// The explicitly configured proxy is trusted to route public domain names;
/// literal private addresses and local hostnames still cannot enter this path.
pub(super) fn validate_proxy_target(url: &url::Url) -> SdkResult<()> {
    let public = match url.host() {
        Some(url::Host::Ipv4(ip)) => is_public_address(ip.into()),
        Some(url::Host::Ipv6(ip)) => is_public_address(ip.into()),
        Some(url::Host::Domain(_)) => public_hostname(url),
        None => false,
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PluginError::invalid_params(
            "web requests require HTTP(S) without URL credentials",
        ));
    }
    if !public {
        return Err(PluginError::from_kind(
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
            "web proxy requests require a public destination hostname or IP address",
        ));
    }
    Ok(())
}

fn public_hostname(url: &url::Url) -> bool {
    let Some(url::Host::Domain(host)) = url.host() else {
        return false;
    };
    let host = host.trim_end_matches('.');
    host.contains('.')
        && !["localhost", "local", "lan", "internal", "corp", "home.arpa"]
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

pub(super) fn validate_addresses(
    url: &url::Url,
    addresses: impl IntoIterator<Item = SocketAddr>,
    config: &WebNetworkConfig,
) -> SdkResult<()> {
    let ranges = ranges(config)?;
    let allow_fake = public_hostname(url);
    for address in addresses {
        let normalized = match address.ip() {
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip.into()),
            ip => ip,
        };
        if !is_public_address(address.ip())
            && !(allow_fake && ranges.iter().any(|range| range.contains(&normalized)))
        {
            let synthetic = match normalized {
                IpAddr::V4(ip) => ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19),
                IpAddr::V6(ip) => ip.is_unique_local(),
            };
            let hint = if allow_fake && synthetic {
                "; if a trusted local TUN proxy supplies Fake-IP DNS, configure its exact synthetic CIDRs in agena.web network.fake_ip_ranges"
            } else {
                ""
            };
            return Err(PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                format!(
                    "web URL host `{}` resolves to non-public address `{address}`{hint}",
                    url.host_str().unwrap_or_default(),
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_dns_accepts_public_hosts_without_local_dns_but_preserves_private_url_denials() {
        for raw in [
            "https://does-not-resolve.invalid/",
            "https://1.1.1.1/",
            "https://[2606:4700:4700::1111]/",
        ] {
            assert!(
                validate_proxy_target(&url::Url::parse(raw).unwrap()).is_ok(),
                "{raw}"
            );
        }
        for raw in [
            "http://localhost/",
            "http://a.local/",
            "http://a.internal/",
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://198.18.0.52/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fdfe:dcba:9876::59]/",
            "http://user:password@example.com/",
            "file:///etc/passwd",
        ] {
            assert!(
                validate_proxy_target(&url::Url::parse(raw).unwrap()).is_err(),
                "{raw}"
            );
        }
    }

    fn configured() -> WebNetworkConfig {
        WebNetworkConfig {
            proxy: None,
            fake_ip_ranges: vec!["198.18.0.0/16".into(), "fdfe:dcba:9876::/64".into()],
        }
    }

    fn check(raw: &str, ips: &[&str], config: &WebNetworkConfig) -> SdkResult<()> {
        validate_addresses(
            &url::Url::parse(raw).unwrap(),
            ips.iter()
                .map(|ip| SocketAddr::new(ip.parse().unwrap(), 443)),
            config,
        )
    }

    #[test]
    fn synthetic_dns_requires_explicit_configuration_for_each_address_family() {
        let url = "https://html.duckduckgo.com/html/";
        let fake = ["198.18.0.52", "fdfe:dcba:9876::59"];
        assert!(check(url, &fake, &WebNetworkConfig::default()).is_err());
        assert!(check(url, &fake, &configured()).is_ok());
        assert!(check(url, &["::ffff:198.18.0.52"], &configured()).is_ok());
        let v4_only = WebNetworkConfig {
            proxy: None,
            fake_ip_ranges: vec!["198.18.0.0/16".into()],
        };
        assert!(check(url, &fake, &v4_only).is_err());
        assert!(check(url, &["198.19.0.52"], &configured()).is_err());
        assert!(check(url, &["fdfe:dcba:9877::59"], &configured()).is_err());
        assert!(
            check(
                url,
                &["1.1.1.1", "2606:4700:4700::1111"],
                &WebNetworkConfig::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn configured_fake_dns_never_authorizes_literals_local_names_or_private_answers() {
        for raw in [
            "http://198.18.0.52/",
            "http://[fdfe:dcba:9876::59]/",
            "http://[::ffff:198.18.0.52]/",
            "http://localhost/",
            "http://localhost./",
            "http://printer/",
            "http://host.local/",
            "http://host.lan/",
            "http://host.internal/",
            "http://host.corp/",
            "http://host.home.arpa/",
        ] {
            assert!(
                check(raw, &["198.18.0.52"], &configured()).is_err(),
                "{raw}"
            );
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "192.0.2.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
        ] {
            let error =
                check("https://example.com/", &["198.18.0.52", ip], &configured()).unwrap_err();
            assert_eq!(
                error.kind,
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                "{ip}"
            );
        }
    }

    #[test]
    fn synthetic_configuration_cannot_whitelist_real_private_or_broad_ranges() {
        for range in [
            "0.0.0.0/0",
            "198.18.0.0/14",
            "198.18.0.1/16",
            "10.0.0.0/8",
            "127.0.0.0/8",
            "192.168.0.0/16",
            "fc00::/7",
            "fdfe:dcba:9876::/48",
            "fdfe:dcba:9876::1/64",
            "::/0",
            "::1/128",
            "2606:4700::/64",
            "not-a-cidr",
        ] {
            assert!(
                validate_config(&WebNetworkConfig {
                    proxy: None,
                    fake_ip_ranges: vec![range.into()]
                })
                .is_err(),
                "{range}"
            );
        }
        assert!(validate_config(&configured()).is_ok());
    }
}
