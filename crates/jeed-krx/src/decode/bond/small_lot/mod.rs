//! 소액채권 — `01M` 장내소액채권 (국민주택채권·지역개발채권 등).
//!
//! ```text
//! B6  IFMSRPD0024   882 B  우선호가
//! A3  IFMSRPD0027   223 B  체결            → bond::trade (세 시장 공통)
//! G7  IFMSRPD0030  1063 B  체결 + 우선호가
//! ```
//!
//! Same 41-byte header, same trade block, same six fields per level as
//! 일반채권 — and after each level, the same six fields **again** for the
//! 채권종류 the instrument belongs to:
//!
//! ```text
//! level n (156 B)
//!   [0:78]    종목   가격11×2 잔량15×2 수익률13×2   → the record's book
//!   [78:156]  종류   가격11×2 잔량15×2 수익률13×2   ← no wire slot
//! tail (61 B)
//!   채권매도/매수호가총잔량 · 채권종류매도/매수호가총잔량 · 0xFF
//! ```
//!
//! So the message is not a different book, it is the same book with a wider
//! stride: [`bond::fill_book`] walks it with [`LEVEL_LEN`] = 156 instead of
//! 78 and reads the first 78 bytes of each level exactly as it does on
//! `IFMSRPD0023`.
//!
//! ## The 채권종류 block is not carried
//!
//! 소액채권 are issued monthly and every issue of the same kind and month is
//! one 채권종류; the exchange aggregates orders across the 종류 and sends that
//! aggregate next to the instrument's own book. It has no identity of its
//! own in the message — no 종류 code, only the instrument's ISIN — so there
//! is no `(venue, symbol)` a second record could carry it under, and the
//! wire record has one book. It is dropped the way 채권호가총잔량 is, and for
//! the same reason: nothing on the wire can say what it is
//! (`documents/todo.md` §10).

pub mod quote;
pub mod trade_quote;

use crate::decode::bond::{self, QTY_LEN};

/// Bytes per book level: the instrument's block followed by the 채권종류
/// block, each the six fields [`bond::LEVEL_LEN`] describes.
pub const LEVEL_LEN: usize = 2 * bond::LEVEL_LEN;

const _: () = assert!(LEVEL_LEN == 156);

/// Bytes after the last level block — four 총잔량 and `0xFF`.
pub const BOOK_TAIL_LEN: usize = 4 * QTY_LEN + 1;

const _: () = assert!(BOOK_TAIL_LEN == 61);
