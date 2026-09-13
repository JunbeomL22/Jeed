//! Tests for `src/recv/endpoint.rs`.

use jeed_fix::recv::{Endpoint, EndpointError};

#[test]
fn host_and_port_round_trip_through_text() {
    let e: Endpoint = "fix.example.com:9876".parse().unwrap();
    assert_eq!(e, Endpoint::new("fix.example.com", 9876));
    assert_eq!(e.to_string(), "fix.example.com:9876");

    // A literal address is a host like any other; nothing resolves at parse.
    let e: Endpoint = "10.20.30.40:9876".parse().unwrap();
    assert_eq!(e.host, "10.20.30.40");
    assert_eq!(e.port, 9876);

    // IPv6 is bracketed, as everywhere else.
    let e: Endpoint = "[::1]:9876".parse().unwrap();
    assert_eq!(e.host, "::1");
    assert_eq!(e.port, 9876);

    assert_eq!("  fix.example.com:9876  ".parse::<Endpoint>().unwrap().port, 9876);
}

#[test]
fn a_missing_or_impossible_port_is_named() {
    // There is no well-known FIX port, so there is nothing to default to.
    assert_eq!("fix.example.com".parse::<Endpoint>(), Err(EndpointError::MissingPort));
    assert_eq!("fix.example.com:".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!("fix.example.com:0".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!("fix.example.com:99999".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!("fix.example.com:x".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!(":9876".parse::<Endpoint>(), Err(EndpointError::EmptyHost));
    assert_eq!("[::1:9876".parse::<Endpoint>(), Err(EndpointError::Unbracketed));
}

#[test]
fn loopback_resolves_and_a_nonsense_host_does_not() {
    let e = Endpoint::new("127.0.0.1", 9876);
    let addr = e.resolve().expect("a literal address resolves to itself");
    assert_eq!(addr.port(), 9876);
    assert!(addr.is_ipv4());

    assert!(Endpoint::new("no-such-host.invalid", 9876).resolve().is_err());
}

#[test]
fn errors_read_as_sentences() {
    for e in [
        EndpointError::MissingPort,
        EndpointError::BadPort,
        EndpointError::EmptyHost,
        EndpointError::Unbracketed,
    ] {
        let text = e.to_string();
        assert!(!text.is_empty());
        assert!(text.chars().next().unwrap().is_lowercase() || text.starts_with('`'));
    }
}
