//! `jeed_krx::recv::endpoint`.

use jeed_krx::recv::{Endpoint, EndpointError};
use std::net::Ipv4Addr;

#[test]
fn a_group_and_port_parse() {
    let e: Endpoint = "233.38.231.92:10302".parse().unwrap();
    assert_eq!(e.group, Ipv4Addr::new(233, 38, 231, 92));
    assert_eq!(e.port, 10302);
    assert!(e.interface.is_unspecified(), "no NIC named means let the route decide");
}

#[test]
fn the_interface_is_part_of_the_address() {
    // A join without one goes out whatever the routing table prefers, which on
    // a box with a management NIC and a feed NIC silently produces no data.
    let e: Endpoint = "233.38.231.92:10302@10.20.30.40".parse().unwrap();
    assert_eq!(e.interface, Ipv4Addr::new(10, 20, 30, 40));
    assert_eq!(e.port, 10302);
}

#[test]
fn a_unicast_address_is_refused() {
    // It parses as an address and then never receives anything, which is the
    // failure mode worth catching at startup rather than at 09:00.
    let err = "10.20.30.40:10302".parse::<Endpoint>().unwrap_err();
    assert_eq!(err, EndpointError::NotMulticast { group: Ipv4Addr::new(10, 20, 30, 40) });
}

#[test]
fn a_missing_port_is_refused() {
    assert!(matches!("233.38.231.92".parse::<Endpoint>(), Err(EndpointError::Address(_))));
}

#[test]
fn a_bad_interface_is_refused() {
    let err = "233.38.231.92:10302@not-an-ip".parse::<Endpoint>().unwrap_err();
    assert!(matches!(err, EndpointError::Interface(_)));
}

#[test]
fn display_round_trips_both_forms() {
    for text in ["233.38.231.92:10302", "233.38.231.92:10302@10.20.30.40"] {
        let e: Endpoint = text.parse().unwrap();
        assert_eq!(e.to_string(), text);
        assert_eq!(text.parse::<Endpoint>().unwrap(), e);
    }
}

#[test]
fn surrounding_space_is_ignored() {
    let e: Endpoint = "  233.38.231.92:10302 @ 10.20.30.40 ".parse().unwrap();
    assert_eq!(e.port, 10302);
    assert_eq!(e.interface, Ipv4Addr::new(10, 20, 30, 40));
}

#[test]
fn on_names_the_interface_without_reparsing() {
    let e = Endpoint::new(Ipv4Addr::new(233, 38, 231, 92), 10302).on(Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(e.to_string(), "233.38.231.92:10302@10.0.0.1");
}
