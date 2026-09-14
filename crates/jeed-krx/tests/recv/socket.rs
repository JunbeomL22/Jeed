//! `jeed_krx::recv::socket` — Winsock and BSD sockets.
//!
//! These open real sockets, on an administratively scoped group that goes
//! nowhere, joined on loopback. They check the parts that are ours — the
//! set-up order, the `WouldBlock` contract, the empty-poller case — not that
//! multicast routing works on the machine running the tests.
//!
//! Loopback is not decoration. On Windows a UDP socket bound to `INADDR_ANY`
//! raises the firewall's "allow access" prompt for each freshly built test
//! binary, and a managed box cannot answer it; `127.0.0.1` is exempt.
//!
//! They are deliberately *not* split by platform: what the caller is promised
//! is the same on both, and the one place the kernels genuinely differ is the
//! bind, which [`two_groups_can_share_a_port`] pins down from the outside.

use jeed_krx::recv::{Endpoint, FeedSocket, Poller, SocketOptions};
use std::net::Ipv4Addr;

/// 239.0.0.0/8 is administratively scoped — nothing forwards it off the host.
fn group(port: u16) -> Endpoint {
    Endpoint::new(Ipv4Addr::new(239, 255, 77, 88), port).on(Ipv4Addr::LOCALHOST)
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
fn two_groups_can_share_a_port() {
    // The bind is where the two kernels part company, and the reason the
    // duplicate-port rule exists.
    //
    // Measured: Windows returns WSAEADDRNOTAVAIL for a bind to the group
    // address, so the socket binds to the interface and the destination
    // address takes no part in the demultiplexing — two endpoints on one port
    // would each receive *both* streams. Linux accepts the group bind, so
    // there each socket gets only its own group.
    //
    // `Receiver::new` refuses two sockets on one port on both platforms: the
    // stricter of the two rules, so one conf file is valid on either. What is
    // pinned down here is only what the socket layer itself promises — the
    // bind succeeds and a second socket on the same port may exist.
    let a = Endpoint::new(Ipv4Addr::new(239, 255, 77, 88), 30_885).on(Ipv4Addr::LOCALHOST);
    let b = Endpoint::new(Ipv4Addr::new(239, 255, 77, 89), 30_885).on(Ipv4Addr::LOCALHOST);
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
