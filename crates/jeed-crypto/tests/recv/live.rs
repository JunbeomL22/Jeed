//! The TLS path, for real — against Binance, and ignored by default.
//!
//! Everything else in this tree runs on loopback without TLS, because
//! `src/recv/link.rs` does not know which stream it is on. That leaves one
//! thing untested: that `rustls` with Mozilla's roots actually completes a
//! handshake with a venue and the frames that follow are what the decoder
//! expects. This is that test. It needs the network and a venue that is up,
//! so it is `#[ignore]` and run by hand:
//!
//! ```text
//! cargo test -p jeed-crypto --test recv live -- --ignored --nocapture
//! ```

use crate::router::Fake;
use crate::sink::Collect;
use jeed_crypto::recv::{Config, Endpoint, LinkState, Receiver, Tls};
use jeed_wire::WireKind;
use std::time::{Duration, Instant};

#[test]
#[ignore = "needs the network and Binance"]
fn binance_spot_trades_arrive_over_tls() {
    let endpoint: Endpoint = "wss://stream.binance.com:9443/ws/btcusdt@trade".parse().unwrap();
    let cfg = Config { record_heartbeat_ns: 0, ..Config::default() };
    let mut rx = Receiver::new(cfg, endpoint, Tls::new(), Fake::new(), Collect::new());

    let deadline = Instant::now() + Duration::from_secs(30);
    while rx.pipeline().sink().len() < 3 {
        assert!(Instant::now() < deadline, "no trades in 30 s; stats: {:?}", rx.stats());
        if let Some(e) = rx.poll_once() {
            eprintln!("link: {e}");
        }
    }
    assert_eq!(rx.state(), LinkState::Open);
    let rec = rx.pipeline().sink().last();
    assert_eq!(rec.kind(), Ok(WireKind::Trade));
    let trade = rec.trade().expect("a trade");
    assert!(trade.price > 0);
    eprintln!("BTCUSDT {} × {} — stats: {:?}", trade.price, trade.qty, rx.stats());
    rx.shutdown(jeed_crypto::recv::CLOSE_NORMAL);
}
