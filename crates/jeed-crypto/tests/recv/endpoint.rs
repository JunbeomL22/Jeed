//! Tests for `src/recv/endpoint.rs`.

use jeed_crypto::recv::{Endpoint, EndpointError};

fn parse(s: &str) -> Endpoint {
    s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
}

#[test]
fn a_full_url_comes_apart() {
    let e = parse("wss://stream.binance.com:9443/stream?streams=btcusdt@trade");

    assert_eq!(e.host, "stream.binance.com");
    assert_eq!(e.port, 9443);
    assert_eq!(e.path, "/stream?streams=btcusdt@trade");
    assert!(e.tls);
}

#[test]
fn wss_defaults_to_443_and_ws_to_80() {
    assert_eq!(parse("wss://ws.okx.com/ws/v5/public").port, 443);
    assert_eq!(parse("ws://localhost/x").port, 80);
    assert!(!parse("ws://localhost/x").tls);
}

#[test]
fn no_path_means_the_root() {
    assert_eq!(parse("wss://api.upbit.com").path, "/");
    assert_eq!(parse("wss://api.upbit.com:443").path, "/");
}

#[test]
fn an_ipv6_literal_is_bracketed() {
    let e = parse("ws://[::1]:9001/feed");
    assert_eq!(e.host, "::1");
    assert_eq!(e.port, 9001);

    let e = parse("ws://[::1]/feed");
    assert_eq!(e.port, 80);
}

#[test]
fn surrounding_whitespace_is_ignored() {
    assert_eq!(parse("  wss://a.b/c \n").host, "a.b");
}

#[test]
fn an_https_url_is_refused_not_guessed_at() {
    assert_eq!("https://api.binance.com/api/v3/depth".parse::<Endpoint>(), Err(EndpointError::Scheme));
    assert_eq!("stream.binance.com:9443".parse::<Endpoint>(), Err(EndpointError::Scheme));
}

#[test]
fn bad_ports_and_hosts_are_refused() {
    assert_eq!("wss://a:0/".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!("wss://a:70000/".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!("wss://a:x/".parse::<Endpoint>(), Err(EndpointError::BadPort));
    assert_eq!("wss:///path".parse::<Endpoint>(), Err(EndpointError::EmptyHost));
    assert_eq!("wss://[::1/".parse::<Endpoint>(), Err(EndpointError::Unbracketed));
    assert_eq!("wss://[::1]x/".parse::<Endpoint>(), Err(EndpointError::BadPort));
}

#[test]
fn display_round_trips_through_parse() {
    for s in ["wss://stream.binance.com:9443/ws/btcusdt@trade", "ws://[::1]:9001/feed", "wss://a.b:443/"] {
        let e = parse(s);
        assert_eq!(e.to_string(), s);
        assert_eq!(parse(&e.to_string()), e);
    }
}

#[test]
fn loopback_resolves_to_itself() {
    let e = Endpoint::new("127.0.0.1", 9001, "/", false);
    assert_eq!(e.resolve().expect("resolves").to_string(), "127.0.0.1:9001");
}
