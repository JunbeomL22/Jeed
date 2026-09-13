//! Tests for `src/recv/http.rs` — one request against a loopback server
//! that answers the three ways a body can be framed.

use jeed_crypto::recv::http::{HttpError, fetch};
use jeed_crypto::recv::{HttpRequest, Tls};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::Duration;

fn tls() -> &'static Tls {
    static TLS: OnceLock<Tls> = OnceLock::new();
    TLS.get_or_init(Tls::new)
}

/// Answers one connection with `response` and hands back the request head.
fn serve(response: &'static [u8]) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "the client hung up before sending a request");
            head.extend_from_slice(&chunk[..n]);
        }
        stream.write_all(response).unwrap();
        stream.flush().unwrap();
        let _ = stream.shutdown(Shutdown::Write);
        String::from_utf8(head).unwrap()
    });
    (base, handle)
}

const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn a_content_length_body_is_read_whole() {
    let (base, server) = serve(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\n\r\n{\"id\":123456}");
    let r = fetch(tls(), &HttpRequest::get(format!("{base}/api/v4/spot/order_book?currency_pair=BTC_USDT")), TIMEOUT).unwrap();
    assert!(r.ok());
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"{\"id\":123456}");

    let head = server.join().unwrap();
    assert!(head.starts_with("GET /api/v4/spot/order_book?currency_pair=BTC_USDT HTTP/1.1\r\n"), "{head}");
    assert!(head.contains("\r\nHost: 127.0.0.1:"), "{head}");
    assert!(head.contains("\r\nConnection: close\r\n"), "{head}");
}

#[test]
fn a_chunked_body_is_joined() {
    let (base, server) = serve(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"code\"\r\na\r\n:\"200000\"}\r\n0\r\n\r\n",
    );
    let r = fetch(tls(), &HttpRequest::post(format!("{base}/api/v1/bullet-public")), TIMEOUT).unwrap();
    assert_eq!(r.body, b"{\"code\":\"200000\"}");
    let head = server.join().unwrap();
    assert!(head.starts_with("POST /api/v1/bullet-public HTTP/1.1\r\n"), "{head}");
    assert!(head.contains("\r\nContent-Length: 0\r\n"), "{head}");
}

#[test]
fn a_body_with_no_framing_ends_with_the_stream() {
    let (base, server) = serve(b"HTTP/1.1 200 OK\r\n\r\n[1,2,3]");
    let r = fetch(tls(), &HttpRequest::get(format!("{base}/")), TIMEOUT).unwrap();
    assert_eq!(r.body, b"[1,2,3]");
    server.join().unwrap();
}

#[test]
fn a_failing_status_comes_back_as_a_status() {
    let (base, server) = serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}");
    let r = fetch(tls(), &HttpRequest::get(format!("{base}/missing")), TIMEOUT).unwrap();
    assert!(!r.ok());
    assert_eq!(r.status, 404);
    server.join().unwrap();
}

#[test]
fn a_chunked_body_cut_short_is_malformed() {
    let (base, server) = serve(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"code\"\r\n");
    let e = fetch(tls(), &HttpRequest::get(format!("{base}/")), TIMEOUT).unwrap_err();
    assert!(matches!(e, HttpError::Malformed(_)), "{e}");
    server.join().unwrap();
}

#[test]
fn a_bad_url_is_refused_before_connecting() {
    let e = fetch(tls(), &HttpRequest::get("wss://api.kucoin.com/x"), TIMEOUT).unwrap_err();
    assert!(matches!(e, HttpError::Url(_)), "{e}");
    let e = fetch(tls(), &HttpRequest::get("https://:1/"), TIMEOUT).unwrap_err();
    assert!(matches!(e, HttpError::Url(_)), "{e}");
}

#[test]
fn nobody_listening_is_a_link_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let e = fetch(tls(), &HttpRequest::get(format!("http://127.0.0.1:{port}/")), TIMEOUT).unwrap_err();
    assert!(matches!(e, HttpError::Link(_)), "{e}");
}
