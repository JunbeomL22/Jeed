//! `jeed::pcap` — a capture through the KRX pipeline.
//!
//! The capture is synthesised here, byte for byte: a libpcap header, Ethernet,
//! IPv4, UDP, and the decoder tests' own 전문 builder for the payload.

#[allow(unused_imports)]
#[path = "../../jeed-krx/tests/decode/common/mod.rs"]
mod builders;

use builders::{B6, RECV_NS, VENUE_NS, kospi200_book};
use jeed::pcap::{Datagram, MAGIC_MICROS, MAGIC_NANOS, PcapError, Reader, Replay, Skip, datagram};
use jeed_krx::TrCode;
use jeed_krx::recv::{IsinFilter, Outcome, TrCodeFilter};
use std::io::Cursor;

const GROUP: [u8; 4] = [233, 38, 231, 92];
const PORT: u16 = 13302;

/// An Ethernet frame carrying one UDP datagram to `dst:port`.
fn frame(dst: [u8; 4], port: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&[0x01, 0x00, 0x5e, 0x26, 0xe7, 0x5c]); // multicast MAC
    f.extend_from_slice(&[0xc8, 0x60, 0x8f, 0x15, 0xaa, 0x41]);
    f.extend_from_slice(&[0x08, 0x00]);
    let ip_len = 20 + 8 + payload.len();
    f.extend_from_slice(&[0x45, 0x00]);
    f.extend_from_slice(&(ip_len as u16).to_be_bytes());
    f.extend_from_slice(&[0x5f, 0xf4, 0x40, 0x00, 0x7b, 17, 0, 0]); // id, DF, ttl, udp, checksum
    f.extend_from_slice(&[192, 168, 21, 10]);
    f.extend_from_slice(&dst);
    f.extend_from_slice(&30001u16.to_be_bytes());
    f.extend_from_slice(&port.to_be_bytes());
    f.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(payload);
    f
}

/// The same frame behind an 802.1Q tag.
fn tagged(frame: &[u8]) -> Vec<u8> {
    let mut f = frame[..12].to_vec();
    f.extend_from_slice(&[0x81, 0x00, 0x00, 0x64]);
    f.extend_from_slice(&frame[12..]);
    f
}

/// A classic libpcap file (little-endian, microseconds, Ethernet).
fn pcap(packets: &[(u64, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC_MICROS.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&262_144u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    for (ts_ns, bytes) in packets {
        out.extend_from_slice(&((ts_ns / 1_000_000_000) as u32).to_le_bytes());
        out.extend_from_slice(&(((ts_ns % 1_000_000_000) / 1_000) as u32).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

fn code(s: &str) -> TrCode {
    TrCode::from_message(s.as_bytes()).unwrap()
}

#[test]
fn a_capture_replays_through_the_pipeline() {
    let good = B6::kospi200(kospi200_book()).build();
    let mut bad_end = good.clone();
    *bad_end.last_mut().unwrap() = 0x00;
    let cut = good[..300].to_vec();
    let mut master = b"A001F".to_vec();
    master.extend_from_slice(&[b'0'; 100]);
    master.push(0xff);

    let f_good = frame(GROUP, PORT, &good);
    let f_bad_end = frame(GROUP, PORT, &bad_end);
    let f_cut = frame(GROUP, PORT, &cut);
    let f_master = frame([233, 38, 231, 93], 13315, &master);
    let f_tagged = tagged(&f_good);
    let arp = {
        let mut f = f_good[..12].to_vec();
        f.extend_from_slice(&[0x08, 0x06]);
        f.extend_from_slice(&[0; 28]);
        f
    };
    let tcp = {
        let mut f = f_good.clone();
        f[23] = 6;
        f
    };
    let fragment = {
        let mut f = f_good.clone();
        f[20] = 0x20; // more fragments
        f
    };

    let file = pcap(&[
        (RECV_NS, &f_good),
        (RECV_NS + 1_000, &f_bad_end),
        (RECV_NS + 2_000, &f_cut),
        (RECV_NS + 3_000, &f_master),
        (RECV_NS + 4_000, &arp),
        (RECV_NS + 5_000, &tcp),
        (RECV_NS + 6_000, &fragment),
        (RECV_NS + 7_000, &f_tagged),
    ]);

    let mut reader = Reader::new(Cursor::new(file)).unwrap();
    let mut dump = Vec::new();
    let mut replay = Replay::new(TrCodeFilter::new([code("B601F")]), IsinFilter::all(), Some(&mut dump)).unwrap();
    while let Some(p) = reader.next_packet().unwrap() {
        replay.packet(&p).unwrap();
    }
    assert_eq!(reader.packets(), 8);

    let p = *replay.packets();
    assert_eq!(p.packets, 8);
    assert_eq!(p.datagrams, 5, "four B6 frames and the master");
    assert_eq!(p.not_ipv4, 1);
    assert_eq!(p.not_udp, 1);
    assert_eq!(p.fragments, 1);
    assert_eq!(p.short, 0);
    assert_eq!(p.truncated, 0);
    assert_eq!(p.first_ns, RECV_NS);
    assert_eq!(p.last_ns, RECV_NS + 7_000);
    assert_eq!(p.bytes, (324 * 3 + 300 + master.len()) as u64);

    let s = replay.stats();
    assert_eq!(s.received, 5);
    assert_eq!(s.published, 2, "the good frame, plain and tagged");
    assert_eq!(s.decode_failed, 1);
    assert_eq!(s.wrong_length, 1);
    assert_eq!(s.filtered_trcode, 1);

    let b6 = replay.code(code("B601F")).unwrap();
    assert_eq!(b6.received, 4);
    assert_eq!(b6.published, 2);
    assert_eq!(b6.failed, 1);
    assert_eq!(b6.wrong_length, 1);
    assert_eq!(b6.bad_end, 2, "the zeroed one, and the cut one ends wherever the cut fell");
    let mut lengths = b6.lengths.clone();
    lengths.sort();
    assert_eq!(lengths, vec![(300, 1), (324, 3)]);
    assert_eq!(b6.failures.len(), 1);
    assert_eq!(b6.failures[0].count, 1);
    assert!(b6.failures[0].head().starts_with(b"B601F"));
    assert!(matches!(b6.failures[0].error, jeed_krx::KrxError::EndKeyword { found: 0x00 }));

    let a0 = replay.code(code("A001F")).unwrap();
    assert_eq!(a0.received, 1);
    assert_eq!(a0.filtered, 1);
    assert_eq!(a0.published, 0);

    // The most-received code comes first.
    let codes = replay.codes();
    assert_eq!(codes[0].code, code("B601F"));
    assert_eq!(codes[1].code, code("A001F"));

    // Each port is a fact about the circuit the conf cannot check.
    let ports = replay.ports();
    assert_eq!(ports.len(), 2);
    let hot = &ports[&(GROUP, PORT)];
    assert_eq!(hot.received, 4);
    assert_eq!(hot.codes[&code("B601F").as_u64()], 4);
    assert_eq!(ports[&([233, 38, 231, 93], 13315)].codes[&code("A001F").as_u64()], 1);

    let mut report = Vec::new();
    replay.write_report(&mut report).unwrap();
    let report = String::from_utf8(report).unwrap();
    assert!(report.contains("capture: 8 packets · 5 datagrams"), "{report}");
    assert!(report.contains("B601F"), "{report}");
    assert!(report.contains("300×1"), "{report}");
    assert!(report.contains("end keyword is 0x00"), "{report}");
    assert!(report.contains("233.38.231.92:13302"), "{report}");

    replay.finish().unwrap();
    let dump = String::from_utf8(dump).unwrap();
    let lines: Vec<&str> = dump.lines().collect();
    assert_eq!(lines.len(), 3, "a header and two records:\n{dump}");
    assert!(lines[0].starts_with("trcode\tkind\tsymbol"));
    let cols: Vec<&str> = lines[1].split('\t').collect();
    assert_eq!(&cols[..4], &["B601F", "Quote", "KR4101V90009", &VENUE_NS.to_string()]);
    assert_eq!(cols[4], RECV_NS.to_string());
    assert_eq!(cols[6], "5", "depth");
    assert_eq!(cols[7], "2", "price scale");
    // quote_ext_kind quote_ext level_flags, then bid levels from column 12.
    assert_eq!(&cols[12..15], &["93695", "8", "2"], "bid 0: price qty count");
    assert_eq!(&cols[42..45], &["93705", "10", "3"], "ask 0");
    assert_eq!(cols.len(), 12 + 60);
    // The tagged frame decoded to the same record at its own capture time.
    let tagged: Vec<&str> = lines[2].split('\t').collect();
    assert_eq!(tagged[4], (RECV_NS + 7_000).to_string());
    assert_eq!(&tagged[12..], &cols[12..]);
}

#[test]
fn a_datagram_is_reported_by_outcome() {
    let good = B6::kospi200(kospi200_book()).build();
    let mut replay = Replay::<Vec<u8>>::new(TrCodeFilter::new([code("B601F")]), IsinFilter::all(), None).unwrap();
    let d = Datagram { dst: GROUP, port: PORT, payload: &good };
    assert!(matches!(replay.datagram(&d, RECV_NS).unwrap(), Outcome::Published { .. }));
    let d = Datagram { dst: GROUP, port: PORT, payload: b"B6" };
    assert_eq!(replay.datagram(&d, RECV_NS).unwrap(), Outcome::TooShort);
    assert_eq!(replay.packets().datagrams, 2);
    assert_eq!(replay.stats().too_short, 1);
    // Too short for a trcode: counted on the port, under no code.
    assert_eq!(replay.ports()[&(GROUP, PORT)].received, 2);
    assert_eq!(replay.codes().len(), 1);
}

#[test]
fn datagram_unwraps_ethernet_ipv4_udp() {
    let payload = b"B601F hello\xff";
    let f = frame(GROUP, PORT, payload);
    let d = datagram(&f).unwrap();
    assert_eq!(d.dst, GROUP);
    assert_eq!(d.port, PORT);
    assert_eq!(d.payload, payload);

    // Behind a VLAN tag, the same datagram.
    assert_eq!(datagram(&tagged(&f)).unwrap(), d);

    // Ethernet padding past the UDP length is not payload.
    let mut padded = f.clone();
    padded.extend_from_slice(&[0xaa; 20]);
    assert_eq!(datagram(&padded).unwrap().payload, payload);

    let mut arp = f[..12].to_vec();
    arp.extend_from_slice(&[0x08, 0x06, 0, 0]);
    assert_eq!(datagram(&arp), Err(Skip::NotIpv4));

    let mut tcp = f.clone();
    tcp[23] = 6;
    assert_eq!(datagram(&tcp), Err(Skip::NotUdp));

    let mut frag = f.clone();
    frag[21] = 0x05; // fragment offset 5
    assert_eq!(datagram(&frag), Err(Skip::Fragment));

    // Cut inside the UDP header, and cut inside the payload.
    assert_eq!(datagram(&f[..38]), Err(Skip::Short));
    assert_eq!(datagram(&f[..f.len() - 3]), Err(Skip::Short));
    assert_eq!(datagram(&f[..10]), Err(Skip::Short));
}

#[test]
fn the_reader_takes_both_magics_in_both_byte_orders() {
    let f = frame(GROUP, PORT, b"B601F\xff");
    // Nanosecond magic, big-endian.
    let mut file = Vec::new();
    file.extend_from_slice(&MAGIC_NANOS.to_be_bytes());
    file.extend_from_slice(&2u16.to_be_bytes());
    file.extend_from_slice(&4u16.to_be_bytes());
    file.extend_from_slice(&[0; 8]);
    file.extend_from_slice(&65_535u32.to_be_bytes());
    file.extend_from_slice(&1u32.to_be_bytes());
    file.extend_from_slice(&1_786_046_401u32.to_be_bytes());
    file.extend_from_slice(&896_314_500u32.to_be_bytes());
    file.extend_from_slice(&(f.len() as u32).to_be_bytes());
    file.extend_from_slice(&((f.len() + 40) as u32).to_be_bytes());
    file.extend_from_slice(&f);

    let mut r = Reader::new(Cursor::new(file)).unwrap();
    let p = r.next_packet().unwrap().unwrap();
    assert_eq!(p.ts_ns, 1_786_046_401_896_314_500);
    assert_eq!(p.bytes, &f[..]);
    assert_eq!(p.original, f.len() + 40);
    assert!(p.truncated());
    assert!(r.next_packet().unwrap().is_none());
    assert_eq!(r.packets(), 1);

    // Microsecond magic, little-endian, is the synthesiser's own.
    let file = pcap(&[(RECV_NS, &f)]);
    let mut r = Reader::new(Cursor::new(file)).unwrap();
    let p = r.next_packet().unwrap().unwrap();
    assert_eq!(p.ts_ns, RECV_NS);
    assert!(!p.truncated());
}

#[test]
fn a_file_cut_inside_a_packet_is_truncated_after_the_whole_ones() {
    let f = frame(GROUP, PORT, b"B601F\xff");
    let file = pcap(&[(RECV_NS, &f), (RECV_NS + 1, &f)]);
    let cut = &file[..file.len() - 5];
    let mut r = Reader::new(Cursor::new(cut)).unwrap();
    assert!(r.next_packet().unwrap().is_some());
    assert!(matches!(r.next_packet(), Err(PcapError::Truncated { packet: 1 })));

    // Cut inside the second packet's header.
    let cut = &file[..file.len() - f.len() - 3];
    let mut r = Reader::new(Cursor::new(cut)).unwrap();
    assert!(r.next_packet().unwrap().is_some());
    assert!(matches!(r.next_packet(), Err(PcapError::Truncated { packet: 1 })));

    // Cut inside the file header.
    assert!(matches!(Reader::new(Cursor::new(&file[..10])), Err(PcapError::Truncated { packet: 0 })));
}

#[test]
fn a_wrong_magic_or_link_type_is_refused_by_name() {
    let mut file = pcap(&[]);
    file[0] = 0x0a;
    file[1] = 0x0d;
    file[2] = 0x0d;
    file[3] = 0x0a;
    assert!(matches!(Reader::new(Cursor::new(file)), Err(PcapError::Magic(0x0a0d_0d0a))));

    let mut file = pcap(&[]);
    file[20] = 113; // Linux cooked
    assert!(matches!(Reader::new(Cursor::new(file)), Err(PcapError::LinkType(113))));

    let mut file = pcap(&[]);
    file.extend_from_slice(&[0; 8]);
    file.extend_from_slice(&(1u32 << 30).to_le_bytes());
    file.extend_from_slice(&(1u32 << 30).to_le_bytes());
    let mut r = Reader::new(Cursor::new(file)).unwrap();
    assert!(matches!(r.next_packet(), Err(PcapError::Length { packet: 0, captured }) if captured == 1 << 30));
}
