//! `forge-book` - deterministic L2 order-book reconstruction.
//!
//! Our feed (cryptohftdata normalized Binance depth) is an incremental DIFF
//! stream with no periodic snapshots: each [`forge_core::EventKind::BookDelta`]
//! either sets a price level's size (`qty > 0`) or removes it (`qty == 0`).
//! [`OrderBook`] folds that stream into a live bid/ask book, starting empty and
//! self-healing during a short warm-up. A marketable update that crosses the
//! book (a new bid at/above resting asks, or vice-versa) consumes the crossed
//! levels on the opposite side, so the book is never left crossed.

#![forbid(unsafe_code)]

mod book;

pub use book::OrderBook;