//! DNS server metadata from IP Helper; this performs no resolver lookup.
use std::{
    mem::{MaybeUninit, size_of},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use windows_sys::Win32::{
    Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS},
    NetworkManagement::{
        IpHelper::{
            GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST,
            GAA_FLAG_SKIP_UNICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
        },
        Ndis::IfOperStatusUp,
    },
    Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6, SOCKET_ADDRESS,
    },
};

pub(super) fn resolver_addresses() -> Vec<IpAddr> {
    // Microsoft recommends a 15 KB initial buffer. Adapters can change between
    // calls, so retry overflow with the returned size without caching results.
    let mut required_bytes: usize = 15 * 1024;
    for _ in 0..3 {
        if required_bytes == 0 || required_bytes > 16 * 1024 * 1024 {
            return vec![];
        }
        // A byte Vec is not guaranteed to satisfy IP_ADAPTER_ADDRESSES alignment.
        let elements = required_bytes.div_ceil(size_of::<IP_ADAPTER_ADDRESSES_LH>());
        let mut buffer = vec![MaybeUninit::<IP_ADAPTER_ADDRESSES_LH>::zeroed(); elements];
        let mut size = (buffer.len() * size_of::<IP_ADAPTER_ADDRESSES_LH>()) as u32;
        let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let flags = GAA_FLAG_SKIP_UNICAST
            | GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_FRIENDLY_NAME;
        // SAFETY: the output allocation is aligned, writable, and sized by
        // `size`; it remains alive while the returned lists are traversed.
        let result = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                flags,
                std::ptr::null(),
                first,
                &mut size,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW {
            required_bytes = size as usize;
            continue;
        }
        if result != ERROR_SUCCESS {
            return vec![];
        }
        let mut addresses = Vec::new();
        let mut adapter = first;
        while !adapter.is_null() {
            // SAFETY: GetAdaptersAddresses succeeded; its linked-list nodes
            // and DNS socket addresses belong to the still-live output buffer.
            unsafe {
                if (*adapter).OperStatus == IfOperStatusUp {
                    let mut dns = (*adapter).FirstDnsServerAddress;
                    while !dns.is_null() {
                        if let Some(address) = socket_address_ip(&(*dns).Address) {
                            addresses.push(address);
                        }
                        dns = (*dns).Next;
                    }
                }
                adapter = (*adapter).Next;
            }
        }
        return addresses;
    }
    vec![]
}

/// The socket pointer must reference initialized storage of the stated length.
unsafe fn socket_address_ip(socket: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if socket.lpSockaddr.is_null() || socket.iSockaddrLength < size_of::<SOCKADDR>() as i32 {
        return None;
    }
    // SAFETY: the caller guarantees readable socket storage; lengths below
    // prevent accessing a larger address structure than the one supplied.
    unsafe {
        match socket.lpSockaddr.read_unaligned().sa_family {
            AF_INET if socket.iSockaddrLength >= size_of::<SOCKADDR_IN>() as i32 => {
                let address = socket.lpSockaddr.cast::<SOCKADDR_IN>().read_unaligned();
                Some(IpAddr::V4(Ipv4Addr::from(
                    address.sin_addr.S_un.S_addr.to_ne_bytes(),
                )))
            }
            AF_INET6 if socket.iSockaddrLength >= size_of::<SOCKADDR_IN6>() as i32 => {
                let address = socket.lpSockaddr.cast::<SOCKADDR_IN6>().read_unaligned();
                Some(IpAddr::V6(Ipv6Addr::from(address.sin6_addr.u.Byte)))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ipv4_network_bytes_and_ipv6_dns_addresses() {
        let mut ipv4 = SOCKADDR_IN::default();
        ipv4.sin_family = AF_INET;
        ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes([192, 0, 2, 53]);
        let socket = SOCKET_ADDRESS {
            lpSockaddr: std::ptr::addr_of_mut!(ipv4).cast(),
            iSockaddrLength: size_of::<SOCKADDR_IN>() as i32,
        };
        // SAFETY: these socket pointers reference the initialized local structs.
        assert_eq!(
            unsafe { socket_address_ip(&socket) },
            Some("192.0.2.53".parse().unwrap())
        );
        let mut ipv6 = SOCKADDR_IN6::default();
        ipv6.sin6_family = AF_INET6;
        ipv6.sin6_addr.u.Byte = "fe80::53".parse::<Ipv6Addr>().unwrap().octets();
        let socket = SOCKET_ADDRESS {
            lpSockaddr: std::ptr::addr_of_mut!(ipv6).cast(),
            iSockaddrLength: size_of::<SOCKADDR_IN6>() as i32,
        };
        // SAFETY: the pointer and length describe the initialized IPv6 struct.
        assert_eq!(
            unsafe { socket_address_ip(&socket) },
            Some("fe80::53".parse().unwrap())
        );
    }

    #[test]
    fn rejects_null_or_truncated_socket_metadata() {
        // SAFETY: null storage is rejected before any dereference.
        assert!(unsafe { socket_address_ip(&SOCKET_ADDRESS::default()) }.is_none());
        let mut ipv6 = SOCKADDR_IN6::default();
        ipv6.sin6_family = AF_INET6;
        let socket = SOCKET_ADDRESS {
            lpSockaddr: std::ptr::addr_of_mut!(ipv6).cast(),
            iSockaddrLength: size_of::<SOCKADDR>() as i32,
        };
        // SAFETY: the smaller advertised size is readable within this struct.
        assert!(unsafe { socket_address_ip(&socket) }.is_none());
    }
}
