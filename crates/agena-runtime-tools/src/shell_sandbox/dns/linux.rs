use std::net::{IpAddr, Ipv4Addr};

pub(super) fn resolver_addresses() -> Vec<IpAddr> {
    std::fs::read_to_string("/etc/resolv.conf")
        .map(|content| parse_resolv_conf(&content))
        .unwrap_or_default()
}

fn parse_resolv_conf(content: &str) -> Vec<IpAddr> {
    content
        .lines()
        .filter_map(|line| {
            let mut words = line
                .split(['#', ';'])
                .next()
                .unwrap_or_default()
                .split_whitespace();
            if words.next() != Some("nameserver") {
                return None;
            }
            let address = words.next()?.parse::<IpAddr>().ok()?.to_canonical();
            // MXC's Linux network namespace forwards host loopback resolvers
            // through the slirp DNS address. Other platforms retain the real IP.
            Some(if address.is_loopback() {
                IpAddr::V4(Ipv4Addr::new(10, 0, 2, 3))
            } else {
                address
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_ipv6_whitespace_and_comments_without_resolving_names() {
        let addresses = parse_resolv_conf(
            "\
            # nameserver 203.0.113.1\n\
            search example.test\n\
            \tnameserver\t192.0.2.53 # primary\n\
            nameserver 2001:db8::53; secondary\n\
            options attempts:2\n\
            nameserver resolver.example.test\n\
            nameserver 192.0.2.54:53\n\
            nameserver\n",
        );
        assert_eq!(
            addresses,
            vec![
                "192.0.2.53".parse::<IpAddr>().unwrap(),
                "2001:db8::53".parse().unwrap()
            ]
        );
    }

    #[test]
    fn maps_ipv4_ipv6_and_mapped_loopback_resolvers_to_mxc_dns() {
        let addresses = parse_resolv_conf(
            "nameserver 127.0.0.53\nnameserver ::1\nnameserver ::ffff:127.0.0.54\n",
        );
        assert_eq!(addresses, vec![IpAddr::V4(Ipv4Addr::new(10, 0, 2, 3)); 3]);
    }

    #[test]
    fn preserves_nonloopback_resolvers_and_handles_empty_configuration() {
        let addresses = parse_resolv_conf("nameserver 10.12.0.1\nnameserver fe80::53\n");
        assert_eq!(
            addresses,
            vec![
                "10.12.0.1".parse::<IpAddr>().unwrap(),
                "fe80::53".parse().unwrap()
            ]
        );
        assert!(parse_resolv_conf("\n# DNS is unavailable\noptions timeout:1\n").is_empty());
    }
}
