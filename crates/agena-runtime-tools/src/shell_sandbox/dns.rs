//! Read current resolver metadata without issuing DNS queries or caching the
//! result. Every returned endpoint still needs the invocation's network grant.
use agena_domain::NetworkTarget;
use std::net::IpAddr;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

pub(super) fn resolver_targets() -> Vec<NetworkTarget> {
    #[cfg(target_os = "linux")]
    let addresses = linux::resolver_addresses();
    #[cfg(windows)]
    let addresses = windows::resolver_addresses();
    #[cfg(not(any(target_os = "linux", windows)))]
    let addresses = Vec::new();
    targets_for_addresses(addresses)
}

fn targets_for_addresses(addresses: Vec<IpAddr>) -> Vec<NetworkTarget> {
    let mut addresses = addresses
        .into_iter()
        .map(|address| address.to_canonical())
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    addresses
        .into_iter()
        .flat_map(|address| {
            let host = match address {
                IpAddr::V4(address) => address.to_string(),
                IpAddr::V6(address) => format!("[{address}]"),
            };
            ["udp", "tcp"].map(|scheme| {
                format!("{scheme}://{host}:53")
                    .parse()
                    .expect("numeric DNS endpoint")
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_targets_are_deduplicated_and_require_udp_and_tcp_for_both_families() {
        let targets = targets_for_addresses(vec![
            "192.0.2.53".parse().unwrap(),
            "::ffff:192.0.2.53".parse().unwrap(),
            "2001:db8::53".parse().unwrap(),
            "2001:db8::53".parse().unwrap(),
        ]);
        assert_eq!(
            targets
                .iter()
                .map(NetworkTarget::original)
                .collect::<Vec<_>>(),
            vec![
                "udp://192.0.2.53:53",
                "tcp://192.0.2.53:53",
                "udp://[2001:db8::53]:53",
                "tcp://[2001:db8::53]:53",
            ]
        );
    }

    #[test]
    fn missing_resolver_metadata_does_not_invent_a_dns_grant() {
        assert!(targets_for_addresses(vec![]).is_empty());
    }
}
