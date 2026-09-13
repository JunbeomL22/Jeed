//! One HTTP/1.1 request, on the cold path.
//!
//! ```text
//! GET /api/v4/spot/order_book?currency_pair=BTC_USDT HTTP/1.1
//! Host: api.gateio.ws
//! Connection: close
//!                                  →  Response { status, body }
//! ```
//!
//! ## Why this exists, and why it is this small
//!
//! Two venues' diff channels never send a whole book (Gate, KuCoin), and one
//! venue will not say where its socket is until asked over REST (KuCoin's
//! `bullet-public`). Both are one request each, at connect time. An HTTP
//! crate for that would be the workspace's second external dependency — and
//! `hyper`'s tree is longer than this whole crate — for a call that happens a
//! handful of times a day. So: the TLS the WebSocket already has, a request
//! line, four headers, and a reader that understands `Content-Length`,
//! `chunked`, and end-of-stream.
//!
//! **The receive loop never calls this.** A blocking fetch inside the loop
//! would stall the socket for the round-trip; the binary makes the call
//! between rounds, once per open, and hands the body to
//! [`Router::route_rest`](crate::recv::pipeline::Router::route_rest). "One connection = one
//! loop" stays true.
//!
//! ## What it does not do
//!
//! Redirects, keep-alive, compression, authentication. A venue's public
//! market-data endpoint needs none of them, and each would be a place to be
//! wrong. A `3xx` comes back as a status the caller sees.

use crate::clock;
use crate::recv::endpoint::Endpoint;
use crate::recv::link::{LinkError, Stream, Tls};
use crate::recv::route::{HttpRequest, Method};
use core::fmt;
use std::io;
use std::time::Duration;

/// Largest body accepted, in bytes. A 5,000-level Binance book is under a
/// megabyte; eight is past anything a start book can be.
pub const MAX_BODY: usize = 8 << 20;

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,

    /// The body, de-chunked.
    pub body: Vec<u8>,
}

impl Response {
    /// `true` for a `2xx`.
    #[inline]
    pub const fn ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

/// Makes `request`, waiting at most `timeout` for the whole exchange.
///
/// Blocking. `tls` is the same configuration the WebSocket uses, so a venue
/// whose certificate the socket accepts is a venue this accepts.
pub fn fetch(tls: &Tls, request: &HttpRequest, timeout: Duration) -> Result<Response, HttpError> {
    let endpoint = parse_url(&request.url)?;
    let deadline = clock::now_ns().saturating_add(timeout.as_nanos() as u64);

    let mut stream = Stream::open(&endpoint, tls, timeout).map_err(HttpError::Link)?;
    let tcp = stream.tcp();
    tcp.set_nodelay(true).map_err(|e| HttpError::io("set_nodelay", e))?;
    tcp.set_write_timeout(Some(timeout)).map_err(|e| HttpError::io("set_write_timeout", e))?;
    // Short reads so the deadline is checked often; a server that stalls is
    // caught by the deadline, not by a single long read.
    tcp.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|e| HttpError::io("set_read_timeout", e))?;

    let head = request_head(request.method, &endpoint);
    stream.write_all(head.as_bytes()).map_err(|e| HttpError::io("write", e))?;
    stream.flush().map_err(|e| HttpError::io("flush", e))?;

    let mut buf = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        if let Some(response) = parse(&buf)? {
            return Ok(response);
        }
        if clock::now_ns() > deadline {
            return Err(HttpError::Timeout);
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_BODY {
                    return Err(HttpError::TooLarge);
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            // A peer that closes without `close_notify` is the common case
            // after `Connection: close`; rustls reports it as an unexpected
            // EOF, and it is the end of the body all the same.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(HttpError::io("read", e)),
        }
    }

    // End of stream: whatever came is the body, unless the framing says it
    // was cut short.
    finish(&buf)
}

/// The request head, through the blank line.
fn request_head(method: Method, endpoint: &Endpoint) -> String {
    let host = if endpoint.port == endpoint.default_port() {
        endpoint.host.clone()
    } else {
        format!("{}:{}", endpoint.host, endpoint.port)
    };
    format!(
        "{} {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: jeed\r\nAccept: application/json\r\n\
         Connection: close\r\nContent-Length: 0\r\n\r\n",
        method.as_str(),
        endpoint.path
    )
}

/// `https://host[:port]/path?query`, or `http://`.
///
/// Reuses [`Endpoint`] for the parts — it is the same shape with a different
/// scheme — so the host resolves the same way the socket's does.
fn parse_url(url: &str) -> Result<Endpoint, HttpError> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(HttpError::Url("scheme is not http or https"));
    };
    let ws = format!("{}://{rest}", if tls { "wss" } else { "ws" });
    ws.parse::<Endpoint>().map_err(|_| HttpError::Url("host or port unreadable"))
}

/// Tries to read a complete response out of `buf`.
///
/// `Ok(None)` when more bytes are needed. Framing decides completeness: a
/// `Content-Length` that is met, a chunked body whose terminator has
/// arrived. A body with neither is complete only at end of stream, which is
/// [`finish`]'s job.
fn parse(buf: &[u8]) -> Result<Option<Response>, HttpError> {
    let Some(head_len) = find(buf, b"\r\n\r\n") else {
        return Ok(None);
    };
    let (status, headers) = head(&buf[..head_len])?;
    let body = &buf[head_len + 4..];

    if let Some(len) = headers.content_length {
        if len > MAX_BODY {
            return Err(HttpError::TooLarge);
        }
        return Ok(if body.len() >= len {
            Some(Response { status, body: body[..len].to_vec() })
        } else {
            None
        });
    }
    if headers.chunked {
        return Ok(dechunk(body)?.map(|body| Response { status, body }));
    }
    Ok(None)
}

/// The response at end of stream.
fn finish(buf: &[u8]) -> Result<Response, HttpError> {
    let Some(head_len) = find(buf, b"\r\n\r\n") else {
        return Err(HttpError::Malformed("no status line before the stream ended"));
    };
    let (status, headers) = head(&buf[..head_len])?;
    let body = &buf[head_len + 4..];
    if headers.content_length.is_some_and(|len| body.len() < len) || headers.chunked {
        // A chunked body that reached here never saw its terminator.
        return Err(HttpError::Malformed("the body was cut short"));
    }
    Ok(Response { status, body: body.to_vec() })
}

/// The headers this reader acts on.
#[derive(Default)]
struct Headers {
    content_length: Option<usize>,
    chunked: bool,
}

/// Status code and the headers that matter, from the head.
fn head(head: &[u8]) -> Result<(u16, Headers), HttpError> {
    let text = core::str::from_utf8(head).map_err(|_| HttpError::Malformed("head is not ASCII"))?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError::Malformed("not an HTTP/1.x status line"));
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(HttpError::Malformed("status code unreadable"))?;

    let mut headers = Headers::default();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            headers.content_length =
                Some(value.parse().map_err(|_| HttpError::Malformed("Content-Length unreadable"))?);
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.split(',').any(|t| t.trim().eq_ignore_ascii_case("chunked"))
        {
            headers.chunked = true;
        }
    }
    Ok((status, headers))
}

/// Joins a chunked body. `Ok(None)` until the zero-length chunk has arrived.
fn dechunk(mut body: &[u8]) -> Result<Option<Vec<u8>>, HttpError> {
    let mut out = Vec::new();
    loop {
        let Some(eol) = find(body, b"\r\n") else {
            return Ok(None);
        };
        let size_text = core::str::from_utf8(&body[..eol])
            .map_err(|_| HttpError::Malformed("chunk size is not ASCII"))?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| HttpError::Malformed("chunk size unreadable"))?;
        body = &body[eol + 2..];
        if size == 0 {
            // Trailers may follow; the body is complete regardless.
            return Ok(Some(out));
        }
        if body.len() < size + 2 {
            return Ok(None);
        }
        out.extend_from_slice(&body[..size]);
        if out.len() > MAX_BODY {
            return Err(HttpError::TooLarge);
        }
        body = &body[size + 2..];
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A request failed.
#[derive(Debug)]
pub enum HttpError {
    /// The URL could not be read.
    Url(&'static str),

    /// Could not connect, or TLS refused.
    Link(LinkError),

    /// A socket call failed.
    Io {
        /// The step that failed.
        call: &'static str,
        /// What the OS said.
        source: io::Error,
    },

    /// The exchange did not complete within the timeout.
    Timeout,

    /// The body is longer than [`MAX_BODY`].
    TooLarge,

    /// The response is not HTTP this reader accepts.
    Malformed(&'static str),
}

impl HttpError {
    fn io(call: &'static str, source: io::Error) -> Self {
        Self::Io { call, source }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(why) => write!(f, "URL: {why}"),
            Self::Link(e) => write!(f, "{e}"),
            Self::Io { call, source } => write!(f, "{call} failed: {source}"),
            Self::Timeout => write!(f, "timed out"),
            Self::TooLarge => write!(f, "body longer than {MAX_BODY} bytes"),
            Self::Malformed(why) => write!(f, "malformed response: {why}"),
        }
    }
}

impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Link(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
