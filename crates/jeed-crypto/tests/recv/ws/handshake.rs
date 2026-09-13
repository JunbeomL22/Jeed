//! Tests for `src/recv/ws/handshake.rs`.

use jeed_crypto::recv::ws::{ACCEPT_LEN, KEY_LEN, WsError, accept, key, parse_response, request};
use jeed_crypto::recv::Endpoint;

/// RFC 6455 §1.3's worked example.
const RFC_KEY: &[u8; KEY_LEN] = b"dGhlIHNhbXBsZSBub25jZQ==";
const RFC_ACCEPT: &[u8; ACCEPT_LEN] = b"s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

fn ok_response() -> Vec<u8> {
    let mut v = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_vec();
    v.extend_from_slice(RFC_ACCEPT);
    v.extend_from_slice(b"\r\n\r\n");
    v
}

#[test]
fn the_rfc_example_key_produces_the_rfc_example_accept() {
    assert_eq!(&accept(RFC_KEY), RFC_ACCEPT);
}

#[test]
fn the_rfc_nonce_encodes_to_the_rfc_key() {
    // "the sample nonce" — sixteen bytes exactly.
    assert_eq!(&key(b"the sample nonce"), RFC_KEY);
}

#[test]
fn the_request_is_what_the_rfc_shows() {
    let e: Endpoint = "wss://stream.binance.com:9443/ws/btcusdt@trade".parse().unwrap();
    let mut out = [0u8; 512];
    let n = request(&mut out, &e, RFC_KEY).expect("fits");

    let text = std::str::from_utf8(&out[..n]).unwrap();
    assert_eq!(
        text,
        "GET /ws/btcusdt@trade HTTP/1.1\r\n\
         Host: stream.binance.com:9443\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
}

#[test]
fn the_host_header_drops_a_default_port() {
    let e: Endpoint = "wss://ws.okx.com/ws/v5/public".parse().unwrap();
    let mut out = [0u8; 512];
    let n = request(&mut out, &e, RFC_KEY).unwrap();
    let text = std::str::from_utf8(&out[..n]).unwrap();

    assert!(text.contains("\r\nHost: ws.okx.com\r\n"), "{text}");
}

#[test]
fn a_request_that_does_not_fit_is_refused_not_cut() {
    let e: Endpoint = "wss://ws.okx.com/ws/v5/public".parse().unwrap();
    let mut out = [0u8; 40];
    assert_eq!(request(&mut out, &e, RFC_KEY), Err(WsError::NoRoom));
}

#[test]
fn a_correct_response_is_accepted_and_measured() {
    let resp = ok_response();
    assert_eq!(parse_response(&resp, RFC_ACCEPT), Ok(Some(resp.len())));
}

#[test]
fn bytes_after_the_headers_are_not_part_of_the_response() {
    let mut resp = ok_response();
    let end = resp.len();
    resp.extend_from_slice(&[0x81, 0x02, b'{', b'}']);

    assert_eq!(parse_response(&resp, RFC_ACCEPT), Ok(Some(end)), "the first frame stays in the buffer");
}

#[test]
fn an_incomplete_response_asks_for_more() {
    let resp = ok_response();
    for cut in [0, 5, 17, resp.len() - 1] {
        assert_eq!(parse_response(&resp[..cut], RFC_ACCEPT), Ok(None), "cut at {cut}");
    }
}

#[test]
fn header_names_and_token_values_are_case_insensitive() {
    let mut v = b"HTTP/1.1 101 OK\r\nUPGRADE:  WebSocket \r\nconnection: keep-alive, UPGRADE\r\nsec-websocket-accept: ".to_vec();
    v.extend_from_slice(RFC_ACCEPT);
    v.extend_from_slice(b"\r\n\r\n");

    assert_eq!(parse_response(&v, RFC_ACCEPT), Ok(Some(v.len())));
}

#[test]
fn the_accept_value_itself_is_exact() {
    let mut v = b"HTTP/1.1 101 OK\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_vec();
    v.extend_from_slice(b"S3PPLMBITXAQ9KYGZZHZRBK+XOO=");
    v.extend_from_slice(b"\r\n\r\n");

    assert_eq!(parse_response(&v, RFC_ACCEPT), Err(WsError::Accept));
}

#[test]
fn a_wrong_or_missing_accept_fails_the_connection() {
    let wrong = ok_response().into_iter().map(|b| if b == b'+' { b'-' } else { b }).collect::<Vec<_>>();
    assert_eq!(parse_response(&wrong, RFC_ACCEPT), Err(WsError::Accept));

    let missing = b"HTTP/1.1 101 OK\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
    assert_eq!(parse_response(missing, RFC_ACCEPT), Err(WsError::Accept));
}

#[test]
fn a_status_other_than_101_is_reported_with_its_code() {
    let resp = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
    assert_eq!(parse_response(resp, RFC_ACCEPT), Err(WsError::Status(404)));

    let resp = b"HTTP/1.1 429 Too Many Requests\r\n\r\n";
    assert_eq!(parse_response(resp, RFC_ACCEPT), Err(WsError::Status(429)));
}

#[test]
fn missing_upgrade_or_connection_headers_fail() {
    let mut no_upgrade = b"HTTP/1.1 101 OK\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_vec();
    no_upgrade.extend_from_slice(RFC_ACCEPT);
    no_upgrade.extend_from_slice(b"\r\n\r\n");
    assert_eq!(parse_response(&no_upgrade, RFC_ACCEPT), Err(WsError::Upgrade));

    let mut no_conn = b"HTTP/1.1 101 OK\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ".to_vec();
    no_conn.extend_from_slice(RFC_ACCEPT);
    no_conn.extend_from_slice(b"\r\n\r\n");
    assert_eq!(parse_response(&no_conn, RFC_ACCEPT), Err(WsError::Connection));
}

#[test]
fn an_extension_we_did_not_offer_is_refused() {
    // Accepting this would mean the next frame is deflated bytes the decoder
    // reads as JSON.
    let mut v = ok_response();
    v.truncate(v.len() - 2);
    v.extend_from_slice(b"Sec-WebSocket-Extensions: permessage-deflate\r\n\r\n");
    assert_eq!(parse_response(&v, RFC_ACCEPT), Err(WsError::Extension));

    let mut v = ok_response();
    v.truncate(v.len() - 2);
    v.extend_from_slice(b"Sec-WebSocket-Protocol: json\r\n\r\n");
    assert_eq!(parse_response(&v, RFC_ACCEPT), Err(WsError::Subprotocol));
}

#[test]
fn something_that_is_not_http_is_malformed() {
    assert_eq!(parse_response(b"SSH-2.0-OpenSSH\r\n\r\n", RFC_ACCEPT), Err(WsError::MalformedResponse));
    assert_eq!(
        parse_response(b"HTTP/1.1 101 OK\r\nno colon here\r\n\r\n", RFC_ACCEPT),
        Err(WsError::MalformedResponse)
    );
}
