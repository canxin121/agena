use std::{fmt, net::IpAddr, str::FromStr};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parsed network target: host and optional port.
pub struct NetworkTarget {
    original: String,
    host: String,
    port: Option<u16>,
}
impl NetworkTarget {
    pub fn original(&self) -> &str {
        &self.original
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }
}
impl FromStr for NetworkTarget {
    type Err = NetworkTargetParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let original = raw.trim().to_string();
        if original.is_empty() {
            return Err(NetworkTargetParseError::Empty);
        }
        if let Ok(url) = url::Url::parse(&original)
            && let Some(host) = url.host_str()
        {
            return Ok(Self {
                host: normalize_host(host),
                port: url.port_or_known_default(),
                original,
            });
        }
        let (host, port) = split_host_port(&original)?;
        let host = normalize_host(host);
        if host.is_empty() {
            return Err(NetworkTargetParseError::MissingHost(original));
        }
        Ok(Self {
            original,
            host,
            port,
        })
    }
}
impl fmt::Display for NetworkTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(port) => write!(f, "{}:{port}", self.host),
            None => f.write_str(&self.host),
        }
    }
}
#[derive(Debug, Error, Clone, PartialEq, Eq)]
/// Error parsing a network target string.
pub enum NetworkTargetParseError {
    #[error("network target must not be empty")]
    Empty,
    #[error("network target `{0}` is missing a host")]
    MissingHost(String),
    #[error("network target `{target}` has invalid port `{port}")]
    InvalidPort { target: String, port: String },
}
fn normalize_host(host: impl AsRef<str>) -> String {
    let host = host.as_ref().trim().trim_end_matches('.');
    // URL::host_str includes IPv6 brackets, while raw host:port parsing
    // already removes them. Authorize both spellings against one identity.
    let address = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V6(address)) => address
            .to_ipv4_mapped()
            .map_or_else(|| address.to_string(), |address| address.to_string()),
        Ok(address) => address.to_string(),
        Err(_) => host.to_ascii_lowercase(),
    }
}
fn split_host_port(target: &str) -> Result<(&str, Option<u16>), NetworkTargetParseError> {
    if let Some(rest) = target.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        return Ok((
            host,
            tail.strip_prefix(':')
                .map_or(Ok(None), |port| parse_port(target, port))?,
        ));
    }
    if target.matches(':').count() == 1
        && let Some((host, port)) = target.rsplit_once(':')
    {
        return Ok((host, parse_port(target, port)?));
    }
    Ok((target, None))
}

fn parse_port(target: &str, port: &str) -> Result<Option<u16>, NetworkTargetParseError> {
    let port = port.trim();
    if port.is_empty() || port == "*" {
        return Ok(None);
    }
    port.parse()
        .map(Some)
        .map_err(|_| NetworkTargetParseError::InvalidPort {
            target: target.into(),
            port: port.into(),
        })
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn raw_and_url_ipv6_targets_have_the_same_host_identity() {
        for (input, host, port) in [
            ("http://[::1]/", "::1", Some(80)),
            ("[0:0:0:0:0:0:0:1]:80", "::1", Some(80)),
            ("https://[::ffff:127.0.0.1]:8443/", "127.0.0.1", Some(8443)),
            ("[::ffff:7f00:1]:8443", "127.0.0.1", Some(8443)),
            ("::ffff:127.0.0.1", "127.0.0.1", None),
            ("127.0.0.1:8443", "127.0.0.1", Some(8443)),
            ("https://EXAMPLE.COM./", "example.com", Some(443)),
        ] {
            let target: NetworkTarget = input.parse().unwrap();
            assert_eq!(target.host(), host, "{input}");
            assert_eq!(target.port(), port, "{input}");
        }
    }
}
