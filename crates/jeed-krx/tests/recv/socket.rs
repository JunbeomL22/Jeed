//! `jeed_krx::recv::socket` — Winsock.
//!
//! These open real sockets, on an administratively scoped group that goes
//! nowhere. They check the parts that are ours — the set-up order, the
//! `WouldBlock` contract, the empty-poller case — not that multicast routing
//! works on the machine running the tests.

use jeed_krx::recv::{Endpoint, FeedSocket, Poller, SocketOptions};
use std::net::Ipv4Addr;

/// 239.0.0.0/8 is administratively scoped — nothing forwards it off the host.
fn group(port: u16) -> Endpoint {
    Endpoint::new(Ipv4Addr::new(239, 255, 77, 88), port)
}

#[test]
fn a_unicast_group_is_refused_before_any_syscall() {
    let bad = Endpoint::new(Ipv4Addr::new(10, 20, 30, 40), 30_881);
    let err = FeedSocket::join(bad, SocketOptions::default()).unwrap_err();
    assert_eq!(err.call, "join");
    assert_eq!(err.code, 0, "our own check, not one Winsock reported");
}

#[test]
fn a_joined_socket_is_empty_rather_than_blocked() {
    // `WouldBlock` is the normal answer on a spinning loop, so it has to read
    // as "nothing yet" and not as an error.
    let sock = FeedSocket::join(group(30_882), SocketOptions::default()).unwrap();
    let mut buf = [0u8; 2048];
    assert_eq!(sock.recv(&mut buf).unwrap(), None);
}

#[test]
fn the_receive_buffer_is_read_back_because_the_request_can_be_refused() {
    // `SO_RCVBUF` is a request. A granted 8 MB and a refused one are the
    // difference between absorbing a scheduling hiccup and dropping a burst.
    let opts = SocketOptions { recv_buffer_bytes: 4 << 20, ..SocketOptions::default() };
    let sock = FeedSocket::join(group(30_883), opts).unwrap();
    assert!(sock.recv_buffer_bytes().unwrap() > 0);
}

#[test]
fn the_bind_filters_by_port_and_not_by_group() {
    // Measured: Windows returns WSAEADDRNOTAVAIL for a bind to the group
    // address, so the socket binds to INADDR_ANY and two endpoints on one port
    // would each receive both streams. `Receiver::new` refuses that
    // configuration; here we only pin down that the bind itself succeeds and
    // that a second socket on the same port is allowed to exist.
    let a = Endpoint::new(Ipv4Addr::new(239, 255, 77, 88), 30_885);
    let b = Endpoint::new(Ipv4Addr::new(239, 255, 77, 89), 30_885);
    let _sa = FeedSocket::join(a, SocketOptions::default()).unwrap();
    let _sb = FeedSocket::join(b, SocketOptions::default()).unwrap();
}

#[test]
fn an_empty_poller_idles_instead_of_failing() {
    // `WSAPoll` rejects an empty set. A feed configured with no sockets should
    // tick quietly — it still has a heartbeat to write.
    let mut p = Poller::new(&[]);
    assert!(p.is_empty());
    assert_eq!(p.wait(0).unwrap(), 0);
    assert!(!p.is_ready(0));
}

#[test]
fn a_quiet_socket_is_not_ready() {
    let sock = FeedSocket::join(group(30_886), SocketOptions::default()).unwrap();
    let sockets = [sock];
    let mut poller = Poller::new(&sockets);

    assert_eq!(poller.len(), 1);
    assert_eq!(poller.wait(1).unwrap(), 0);
    assert!(!poller.is_ready(0));
}
