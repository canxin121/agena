//! Address policy shared by public web requests, OAuth metadata and TUI images.
//! Fake-IP addresses are routing tokens for a public DNS hostname; they never
//! make a literal IP or a local hostname into a public destination.
use std::net::IpAddr;

pub fn is_public_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.contains('.')
        && host.parse::<IpAddr>().is_err()
        && !["localhost", "local", "lan", "internal", "corp", "home.arpa"]
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

/// Common Clash/Mihomo, sing-box and Surge TUN synthetic DNS pools. Keep this
/// separate from public IP classification: only DNS answers for a public
/// hostname may use these routes, with the original Host and TLS identity.
pub fn is_fake_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, _, _] = address.octets();
            first == 198 && matches!(second, 18 | 19)
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_fake_ip(mapped.into());
            }
            let segments = address.segments();
            // fc00::/18 and fdfe:dcba:9876::/64.
            (segments[0] == 0xfc00 && segments[1] & 0xc000 == 0)
                || segments[..4] == [0xfdfe, 0xdcba, 0x9876, 0]
        }
    }
}

pub fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, third, _] = address.octets();
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_unspecified()
                || address.is_multicast()
                || first == 0
                || first >= 240
                || (first == 100 && (64..=127).contains(&second))
                || (first == 198 && matches!(second, 18 | 19))
                || (first == 192 && second == 0 && third == 0))
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_public_ip(mapped.into());
            }
            let segments = address.segments();
            // Global unicast, excluding documentation, transition and protocol
            // ranges which can also encode private IPv4 destinations.
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && segments[1] < 0x0200)
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_ip_pools_cover_both_address_families_without_becoming_public_ips() {
        for raw in [
            "198.18.0.0",
            "198.19.255.255",
            "::ffff:198.18.1.80",
            "fc00::1",
            "fc00:3fff:ffff:ffff:ffff:ffff:ffff:ffff",
            "fdfe:dcba:9876::1",
            "fdfe:dcba:9876::ffff:ffff:ffff:ffff",
        ] {
            let address = raw.parse().unwrap();
            assert!(is_fake_ip(address), "{raw}");
            assert!(!is_public_ip(address), "{raw}");
        }
        for raw in [
            "198.17.255.255",
            "198.20.0.0",
            "fc00:4000::1",
            "fd00::1",
            "fdfe:dcba:9877::1",
            "fdfe:dcba:9876:1::1",
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "::1",
            "::ffff:192.168.1.1",
            "fe80::1",
            "8.8.8.8",
        ] {
            assert!(!is_fake_ip(raw.parse().unwrap()), "{raw}");
        }
    }

    #[test]
    fn public_destinations_exclude_local_names_and_non_public_address_encodings() {
        for host in ["chatgpt.com", "EXAMPLE.COM.", "does-not-resolve.invalid"] {
            assert!(is_public_hostname(host), "{host}");
        }
        for host in [
            "localhost",
            "LOCALHOST.",
            "printer",
            "host.local",
            "host.lan",
            "host.internal",
            "host.corp",
            "host.home.arpa",
            "198.18.0.1",
            "::ffff:1.1.1.1",
        ] {
            assert!(!is_public_hostname(host), "{host}");
        }
        for raw in [
            "0.1.2.3",
            "100.64.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "224.0.0.1",
            "240.0.0.1",
            "::ffff:127.0.0.1",
            "::",
            "fe80::1",
            "2001:db8::1",
            "2001:2::1",
            "2002:7f00:1::1",
            "64:ff9b::7f00:1",
            "3fff::1",
        ] {
            assert!(!is_public_ip(raw.parse().unwrap()), "{raw}");
        }
        for raw in [
            "1.1.1.1",
            "8.8.8.8",
            "::ffff:1.1.1.1",
            "2606:4700:4700::1111",
        ] {
            assert!(is_public_ip(raw.parse().unwrap()), "{raw}");
        }
    }
}
