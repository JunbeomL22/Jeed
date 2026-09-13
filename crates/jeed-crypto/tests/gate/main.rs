//! `jeed_crypto::gate` — the decoders, against the frames Gate V4 sends.

mod common;
mod delta;
mod snapshot;
mod trade;

#[cfg(feature = "recv")]
mod router;
