//! Fixed-width ASCII numeric parsing for feed decoders.
//!
//! Ported from `fractal-engine`'s `utilities::converters`, unchanged except for
//! the removal of `serde`/`deepsize` derives and the `format`/`rfc3339`/
//! `timestamp` modules, which are output-side and never run on the receive
//! path. It is a port rather than a dependency because `fractal-engine` is the
//! **consumer** of this workspace: a crate dependency in that direction would
//! put the parser on the far side of the process split it exists to make.
//!
//! ## Why not scan for the decimal point
//!
//! The obvious way to read `[sign][5].[2]` is to walk the bytes, accumulate
//! digits, and note where the `.` was. That is one branch per byte, and it
//! returns the scale alongside the value, so the caller carries a `(value,
//! decimals)` pair around.
//!
//! This does neither. [`FixedExtractor`] is told the point's index up front,
//! loads the field as a `u64`/`u128` ([`decimal_core::le_bytes`]), deletes the
//! point with a mask-and-shift ([`decimal_core::squeeze_point`]), validates
//! every digit in one bitwise test ([`decimal_core::check`]), and folds the
//! result eight digits at a time ([`decimal_core::bcd`]). The scale never
//! travels with the value — it belongs to the instrument, so the decoder
//! resolves it once and stamps it on the record header.
//!
//! ## Knowing the point's index without reading the message
//!
//! KRX derivative prices are nine bytes in three shapes and the product group
//! does not settle which: 3개월무위험지표금리선물 shares `06F` with instruments
//! that use `[5].[2]`. The rule that does hold is product group **plus ISIN
//! prefix** — see `jeed_krx::decode::derivative::price_shape`. One comparison
//! per message selects the extractor; the parse itself stays branch-free.

#![deny(missing_docs)]

pub mod decimal_core;
pub mod error;
pub mod extractor;
pub mod integer_parser;

pub use error::{ConfigErr, ParseErr};
pub use extractor::{Config, DynamicExtractor, Extractor, FixedExtractor};
pub use integer_parser::Biscuit;
