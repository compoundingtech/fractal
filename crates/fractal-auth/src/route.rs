use std::net::IpAddr;

/// Whether an address is in Tailscale's ranges: `100.64.0.0/10` or `fd7a:115c:a1e0::/48`.
pub fn is_tailnet_address(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            octets[0] == 100 && octets[1] & 0xc0 == 64
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
    }
}

/// Whether an address may carry replication traffic: loopback or tailnet, never a LAN.
pub fn is_permitted_route_address(address: &IpAddr) -> bool {
    address.is_loopback() || is_tailnet_address(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tailnet_ranges_are_exact() {
        for inside in ["100.64.0.1", "100.127.255.254", "fd7a:115c:a1e0::1"] {
            assert!(is_tailnet_address(&inside.parse().unwrap()), "{inside}");
        }
        for outside in [
            "100.63.255.255",
            "100.128.0.1",
            "192.168.1.10",
            "10.0.0.1",
            "fd7a:115c:a1e1::1",
            "127.0.0.1",
        ] {
            assert!(!is_tailnet_address(&outside.parse().unwrap()), "{outside}");
        }
    }
}
