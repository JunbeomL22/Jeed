//! `jeed::conf::crypto` — reading and validating `conf/crypto.toml`.

use jeed::conf::crypto::{Warning, venue_by_name, venue_name};
use jeed::conf::{ConfError, CryptoConf, RuleError};
use jeed::cpu::Topology;
use jeed::toml::parse;
use jeed_crypto::recv::{Channel, ChannelSet, Mode, RouterError, VenueRouter};
use jeed_wire::Venue;

const MINIMAL: &str = r#"
[[feed]]
name = "binance"
venue = "binance-spot"
mode = "block"
cores = [2]
ring = "jeed.test.binance"
ring_slots = 4096

[[feed.instrument]]
symbol = "BTCUSDT"
price_decimals = 2
qty_decimals = 5
channels = ["trade", "delta"]

[[feed]]
name = "gate"
venue = "gate-spot"
mode = "block"
cores = [4, 5]
ring = "jeed.test.gate"
ring_slots = 256
rest = "http://127.0.0.1:9"

[[feed.instrument]]
symbol = "BTC_USDT"
price_decimals = 1
qty_decimals = 4
channels = ["delta"]
"#;

fn conf(text: &str) -> CryptoConf {
    CryptoConf::from_table(&parse(text).unwrap()).unwrap()
}

#[test]
fn the_minimal_conf_reads_and_validates() {
    let c = conf(MINIMAL);
    assert_eq!(c.feeds.len(), 2);
    let b = &c.feeds[0];
    assert_eq!(b.venue, Venue::BinanceSpot);
    assert_eq!(b.mode, Mode::Block);
    assert_eq!(b.cores, [2]);
    assert_eq!(b.instruments.len(), 1);
    assert_eq!(b.instruments[0].symbol, "BTCUSDT");
    assert_eq!(b.instruments[0].channels, ChannelSet::EMPTY.with(Channel::Trade).with(Channel::Delta));
    assert_eq!(b.instruments[0].depth, None);
    assert_eq!(b.url, None);
    assert_eq!(b.endpoint().unwrap().host, "stream.binance.com");
    assert_eq!(b.ping_secs, 30);
    assert_eq!(b.reconnect_secs, 5);
    assert_eq!(b.burst, 64);
    assert_eq!(c.feeds[1].rest.as_deref(), Some("http://127.0.0.1:9"));
    assert_eq!(c.health.stale_ns, 500_000_000, "absent guard is on");
    assert_eq!(c.report_secs, 10);
    c.validate(None).unwrap();
    assert_eq!(c.warnings(None), [Warning::NoTopology]);
    assert!(c.warnings(Some(&Topology::from_siblings(vec![1, 2, 4, 8, 16, 32]))).is_empty());
}

#[test]
fn the_router_and_the_rest_override_come_from_the_feed() {
    let c = conf(MINIMAL);
    let r = c.feeds[1].router().unwrap();
    assert!(matches!(r, VenueRouter::Gate(_)));
    assert!(jeed_crypto::recv::Router::rest_books(&r)[0].request.url.starts_with("http://127.0.0.1:9/api/v4/"));
    let cfg = c.feeds[0].receiver_config(c.health);
    assert_eq!(cfg.ping_interval_ns, 30_000_000_000);
    assert_eq!(cfg.record_heartbeat_ns, 100_000_000);
}

#[test]
fn every_venue_name_round_trips() {
    for name in jeed::conf::crypto::venue_names() {
        let v = venue_by_name(name).unwrap();
        assert_eq!(venue_name(v), Some(name));
        assert!(VenueRouter::is_crypto(v));
    }
    assert_eq!(venue_by_name("binance"), None);
    assert_eq!(venue_name(Venue::Krx), None);
}

#[test]
fn an_unknown_venue_names_the_known_ones() {
    let e = CryptoConf::from_table(&parse(&MINIMAL.replace("\"gate-spot\"", "\"gate\"")).unwrap()).unwrap_err();
    let msg = e.to_string();
    assert!(msg.starts_with("feed[gate]: `venue` \"gate\" is not a venue; one of binance-spot, "), "{msg}");
    assert!(msg.contains("kucoin-futures"), "{msg}");
}

#[test]
fn an_unknown_key_anywhere_is_refused() {
    let text = MINIMAL.replace("ring_slots = 256", "ring_slots = 256\nring_slot = 256");
    let e = CryptoConf::from_table(&parse(&text).unwrap()).unwrap_err();
    assert!(matches!(&e, ConfError::Unknown { at, key } if at == "feed[gate]" && key == "ring_slot"), "{e}");

    let e = CryptoConf::from_table(&parse(&MINIMAL.replace("qty_decimals = 4", "qty_decimal = 4")).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate].instrument[BTC_USDT]: `qty_decimals` is required");

    let text = MINIMAL.replace("qty_decimals = 4\n", "qty_decimals = 4\nlot = 1\n");
    let e = CryptoConf::from_table(&parse(&text).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate].instrument[BTC_USDT]: unknown key `lot`");
}

#[test]
fn channels_are_checked_when_read() {
    let e = CryptoConf::from_table(&parse(&MINIMAL.replace("[\"delta\"]", "[\"orderbook\"]")).unwrap()).unwrap_err();
    assert!(e.to_string().contains("`orderbook` is not a channel; one of trade, bbo, book, delta"), "{e}");
    let e = CryptoConf::from_table(&parse(&MINIMAL.replace("[\"delta\"]", "[\"delta\", \"delta\"]")).unwrap()).unwrap_err();
    assert!(e.to_string().contains("`delta` is listed twice"), "{e}");
    let e = CryptoConf::from_table(&parse(&MINIMAL.replace("[\"delta\"]", "[]")).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate].instrument[BTC_USDT]: `channels` is empty");
}

#[test]
fn scales_depth_and_urls_are_checked_when_read() {
    let e = CryptoConf::from_table(&parse(&MINIMAL.replace("price_decimals = 1", "price_decimals = 9")).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate].instrument[BTC_USDT]: `price_decimals` must be 0..=8");
    let text = MINIMAL.replace("channels = [\"delta\"]", "channels = [\"delta\"]\ndepth = 0");
    let e = CryptoConf::from_table(&parse(&text).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate].instrument[BTC_USDT]: `depth` must be at least 1");
    let text = MINIMAL.replace("rest = \"http://127.0.0.1:9\"", "url = \"https://api.gateio.ws/ws/v4/\"");
    let e = CryptoConf::from_table(&parse(&text).unwrap()).unwrap_err();
    assert!(e.to_string().starts_with("feed[gate]: `url` `https://api.gateio.ws/ws/v4/`: URL must start with"), "{e}");
    let text = MINIMAL.replace("rest = \"http://127.0.0.1:9\"", "rest = \"api.gateio.ws\"");
    let e = CryptoConf::from_table(&parse(&text).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate]: `rest` `api.gateio.ws` must start with http:// or https://");
    let text = MINIMAL.replace("rest = \"http://127.0.0.1:9\"", "reconnect_secs = 0");
    let e = CryptoConf::from_table(&parse(&text).unwrap()).unwrap_err();
    assert_eq!(e.to_string(), "feed[gate]: `reconnect_secs` must be at least 1");
}

#[test]
fn validate_applies_the_shared_placement_rules() {
    let c = conf(&MINIMAL.replace("cores = [4, 5]", "cores = [2]"));
    assert_eq!(c.validate(None), Err(RuleError::CoreShared { core: 2, a: "binance".into(), b: "gate".into() }));

    let c = conf(&MINIMAL.replace("ring_slots = 256", "ring_slots = 300"));
    assert_eq!(c.validate(None), Err(RuleError::RingSlots { feed: "gate".into(), slots: 300 }));

    let c = conf(&MINIMAL.replace("name = \"gate\"\nvenue = \"gate-spot\"\nmode = \"block\"", "name = \"gate\"\nvenue = \"gate-spot\"\nmode = \"spin\""));
    assert_eq!(c.validate(None), Err(RuleError::SpinOnSeveralCores { feed: "gate".into(), cores: 2 }));

    assert_eq!(CryptoConf::from_table(&parse("").unwrap()).unwrap().validate(None), Err(RuleError::NoFeeds));
}

#[test]
fn validate_refuses_what_the_venue_cannot_serve() {
    let c = conf(&MINIMAL.replace("channels = [\"delta\"]", "channels = [\"book\"]"));
    assert_eq!(
        c.validate(None),
        Err(RuleError::Router {
            feed: "gate".into(),
            source: RouterError::Unsupported { venue: Venue::GateSpot, channel: Channel::Book }
        })
    );
    let c = conf(&MINIMAL.replace("channels = [\"trade\", \"delta\"]", "channels = [\"trade\", \"delta\"]\ndepth = 7"));
    assert_eq!(
        c.validate(None),
        Err(RuleError::Router { feed: "binance".into(), source: RouterError::Depth { venue: Venue::BinanceSpot, depth: 7 } })
    );
    let e = c.validate(None).unwrap_err();
    assert_eq!(e.to_string(), "feed[binance]: BINANCE_SPOT has no book of depth 7");
}

#[test]
fn validate_refuses_a_feed_with_no_instruments_or_a_symbol_the_wire_cannot_hold() {
    let text = MINIMAL.replace(
        "[[feed.instrument]]\nsymbol = \"BTC_USDT\"\nprice_decimals = 1\nqty_decimals = 4\nchannels = [\"delta\"]\n",
        "",
    );
    let c = conf(&text);
    assert_eq!(c.validate(None), Err(RuleError::NoInstruments { feed: "gate".into() }));

    let long = "X".repeat(25);
    let c = conf(&MINIMAL.replace("BTC_USDT", &long));
    let e = c.validate(None).unwrap_err();
    assert!(matches!(&e, RuleError::Instrument { feed, symbol, .. } if feed == "gate" && symbol == &long), "{e}");
    assert!(e.to_string().starts_with("feed[gate]: instrument XXXX"), "{e}");
}

#[test]
fn a_symbol_under_two_feeds_on_one_venue_is_a_warning() {
    let text = MINIMAL.replace("venue = \"gate-spot\"", "venue = \"binance-spot\"").replace("BTC_USDT", "BTCUSDT").replace("rest = \"http://127.0.0.1:9\"\n", "");
    let c = conf(&text);
    c.validate(None).unwrap();
    let w = c.warnings(Some(&Topology::from_siblings(vec![1, 2, 4, 8, 16, 32])));
    assert_eq!(
        w,
        [Warning::SymbolInTwoFeeds { venue: Venue::BinanceSpot, symbol: "BTCUSDT".into(), a: "binance".into(), b: "gate".into() }]
    );
    assert_eq!(
        w[0].to_string(),
        "BINANCE_SPOT BTCUSDT is listed under both \"binance\" and \"gate\"; it will be published to two rings"
    );
}

#[test]
fn kucoin_has_no_endpoint_until_the_ticket() {
    let text = MINIMAL.replace("venue = \"gate-spot\"", "venue = \"kucoin-spot\"").replace("BTC_USDT", "BTC-USDT");
    let c = conf(&text);
    assert_eq!(c.feeds[1].endpoint(), None);
    let text = text.replace("rest = \"http://127.0.0.1:9\"", "url = \"ws://127.0.0.1:9/\"");
    let c = conf(&text);
    assert_eq!(c.feeds[1].endpoint().unwrap().port, 9, "an explicit url is dialled even so");
}

#[test]
fn the_example_conf_validates() {
    let c = CryptoConf::load(std::path::Path::new("../../conf/crypto.example.toml")).unwrap();
    assert_eq!(c.feeds.len(), 4);
    c.validate(None).unwrap();
    assert_eq!(c.warnings(None), [Warning::NoTopology]);
}
