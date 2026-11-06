//! Address policy edges through the public constructor, without network access.
use batter_outbound_http::{Origin, OriginError};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn assert_policy(address: IpAddr, expected: Result<(), OriginError>) {
    let url = match address {
        IpAddr::V4(ip) => format!("https://{ip}"),
        IpAddr::V6(ip) => format!("https://[{ip}]"),
    };
    assert_eq!(
        Origin::public_https(&url).map(|_| ()),
        expected,
        "{address}"
    );
    assert!(Origin::https(&url).is_ok(), "legacy policy: {address}");
}

#[test]
fn ipv4_excluded_ranges_have_exact_edges() {
    // Independent endpoints from the IANA ranges, including the combined
    // multicast/reserved tail. Each existing neighbor is ordinary public IPv4.
    for (first, last) in [
        ("0.0.0.0", "0.255.255.255"),
        ("10.0.0.0", "10.255.255.255"),
        ("100.64.0.0", "100.127.255.255"),
        ("127.0.0.0", "127.255.255.255"),
        ("169.254.0.0", "169.254.255.255"),
        ("172.16.0.0", "172.31.255.255"),
        ("192.0.0.0", "192.0.0.255"),
        ("192.0.2.0", "192.0.2.255"),
        ("192.31.196.0", "192.31.196.255"),
        ("192.52.193.0", "192.52.193.255"),
        ("192.88.99.0", "192.88.99.255"),
        ("192.168.0.0", "192.168.255.255"),
        ("192.175.48.0", "192.175.48.255"),
        ("198.18.0.0", "198.19.255.255"),
        ("198.51.100.0", "198.51.100.255"),
        ("203.0.113.0", "203.0.113.255"),
        ("224.0.0.0", "255.255.255.255"),
    ] {
        let first: Ipv4Addr = first.parse().unwrap();
        let last: Ipv4Addr = last.parse().unwrap();
        assert_policy(first.into(), Err(OriginError::NonPublicAddress));
        assert_policy(last.into(), Err(OriginError::NonPublicAddress));
        if let Some(before) = first.to_bits().checked_sub(1) {
            assert_policy(Ipv4Addr::from_bits(before).into(), Ok(()));
        }
        if let Some(after) = last.to_bits().checked_add(1) {
            assert_policy(Ipv4Addr::from_bits(after).into(), Ok(()));
        }
    }
}

#[test]
fn ipv6_excluded_ranges_have_exact_edges_within_global_unicast() {
    for (first, last) in [
        ("2001::", "2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff"),
        ("2001:db8::", "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff"),
        ("2002::", "2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
        ("2620:4f:8000::", "2620:4f:8000:ffff:ffff:ffff:ffff:ffff"),
        ("3fff::", "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff"),
    ] {
        let first: Ipv6Addr = first.parse().unwrap();
        let last: Ipv6Addr = last.parse().unwrap();
        assert_policy(first.into(), Err(OriginError::NonPublicAddress));
        assert_policy(last.into(), Err(OriginError::NonPublicAddress));
        assert_policy(Ipv6Addr::from_bits(first.to_bits() - 1).into(), Ok(()));
        assert_policy(Ipv6Addr::from_bits(last.to_bits() + 1).into(), Ok(()));
    }
    for (address, expected) in [
        (
            "1fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            Err(OriginError::NonPublicAddress),
        ),
        ("2000::", Ok(())),
        ("3fff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Ok(())),
        ("4000::", Err(OriginError::NonPublicAddress)),
    ] {
        assert_policy(address.parse().unwrap(), expected);
    }
}
