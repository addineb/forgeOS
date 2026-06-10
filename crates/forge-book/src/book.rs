//! The [`OrderBook`] fold and its read views.

use std::collections::BTreeMap;

use forge_core::{Event, EventKind, ForgeError, ForgeResult, Price, Qty, Side};

/// FNV-1a 64-bit constants for the deterministic state hash.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fold_i64(h: &mut u64, x: i64) {
    for b in x.to_le_bytes() {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(FNV_PRIME);
    }
}

/// A live L2 order book reconstructed from a diff stream.
///
/// Levels are kept as `price_raw -> qty_raw` maps (fixed-point integers), so
/// iteration order is deterministic. The bid map max key and the ask map min
/// key are the best quotes. An optional per-side depth cap evicts levels far
/// from the touch (the feed only maintains the visible top-N).
#[derive(Debug, Default, Clone)]
pub struct OrderBook {
    bids: BTreeMap<i64, i64>,
    asks: BTreeMap<i64, i64>,
    last_ts: u64,
    applied: u64,
    crossed_trims: u64,
    evicted: u64,
    /// Max levels kept per side (0 = unlimited).
    max_levels: usize,
}

impl OrderBook {
    /// A fresh, empty, unlimited-depth book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A book that keeps at most `max_levels` price levels per side (the ones
    /// nearest the touch); 0 means unlimited. Use the feed visible depth
    /// (Binance depth20 => 20) to bound memory and keep sweeps fast without
    /// dropping any wall/spoofing-relevant level.
    #[must_use]
    pub fn with_max_levels(max_levels: usize) -> Self {
        Self { max_levels, ..Self::default() }
    }

    /// Apply one event. Book-delta / snapshot rows mutate the book; trades and
    /// quotes are ignored (they carry no level state). Advances `last_ts`.
    ///
    /// # Errors
    /// [`ForgeError::OutOfRange`] if a book-delta carries no side.
    pub fn apply(&mut self, ev: &Event) -> ForgeResult<()> {
        match ev.kind {
            EventKind::BookDelta | EventKind::BookSnapshot => self.apply_level(ev)?,
            EventKind::Trade | EventKind::Quote => {}
        }
        let lt = ev.local_ts.get();
        if lt > self.last_ts {
            self.last_ts = lt;
        }
        Ok(())
    }

    fn apply_level(&mut self, ev: &Event) -> ForgeResult<()> {
        let side = ev
            .side
            .ok_or(ForgeError::OutOfRange { field: "book-delta-side" })?;
        let price = ev.price.raw();
        let qty = ev.qty.raw();
        match side {
            Side::Bid => {
                if qty == 0 {
                    self.bids.remove(&price);
                } else {
                    self.bids.insert(price, qty);
                    self.trim_crossed_asks(price);
                }
            }
            Side::Ask => {
                if qty == 0 {
                    self.asks.remove(&price);
                } else {
                    self.asks.insert(price, qty);
                    self.trim_crossed_bids(price);
                }
            }
        }
        self.enforce_cap(side);
        self.applied += 1;
        Ok(())
    }

    /// A new bid at `bid_price` consumes any resting asks priced <= it.
    fn trim_crossed_asks(&mut self, bid_price: i64) {
        let crossed: Vec<i64> = self.asks.range(..=bid_price).map(|(&p, _)| p).collect();
        for p in crossed {
            self.asks.remove(&p);
            self.crossed_trims += 1;
        }
    }

    /// A new ask at `ask_price` consumes any resting bids priced >= it.
    fn trim_crossed_bids(&mut self, ask_price: i64) {
        let crossed: Vec<i64> = self.bids.range(ask_price..).map(|(&p, _)| p).collect();
        for p in crossed {
            self.bids.remove(&p);
            self.crossed_trims += 1;
        }
    }

    /// Evict the levels furthest from the touch so each side keeps at most
    /// `max_levels` (the near-touch levels where walls/spoofing live).
    fn enforce_cap(&mut self, side: Side) {
        if self.max_levels == 0 {
            return;
        }
        match side {
            Side::Bid => {
                while self.bids.len() > self.max_levels {
                    if let Some((&worst, _)) = self.bids.iter().next() {
                        self.bids.remove(&worst);
                        self.evicted += 1;
                    } else {
                        break;
                    }
                }
            }
            Side::Ask => {
                while self.asks.len() > self.max_levels {
                    if let Some((&worst, _)) = self.asks.iter().next_back() {
                        self.asks.remove(&worst);
                        self.evicted += 1;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Best bid (highest price), if any.
    #[must_use]
    pub fn best_bid(&self) -> Option<(Price, Qty)> {
        self.bids
            .iter()
            .next_back()
            .map(|(&p, &q)| (Price::from_raw(p), Qty::from_raw(q)))
    }

    /// Best ask (lowest price), if any.
    #[must_use]
    pub fn best_ask(&self) -> Option<(Price, Qty)> {
        self.asks
            .iter()
            .next()
            .map(|(&p, &q)| (Price::from_raw(p), Qty::from_raw(q)))
    }

    /// Mid price `(best_bid + best_ask) / 2` (integer division; may truncate a
    /// half-tick). `None` until both sides exist.
    #[must_use]
    pub fn mid(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some((b, _)), Some((a, _))) => Some(Price::from_raw((b.raw() + a.raw()) / 2)),
            _ => None,
        }
    }

    /// Spread `best_ask - best_bid`. `None` until both sides exist.
    #[must_use]
    pub fn spread(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some((b, _)), Some((a, _))) => Some(Price::from_raw(a.raw() - b.raw())),
            _ => None,
        }
    }

    /// Top `n` bids, highest price first.
    #[must_use]
    pub fn bids_top(&self, n: usize) -> Vec<(Price, Qty)> {
        self.bids
            .iter()
            .rev()
            .take(n)
            .map(|(&p, &q)| (Price::from_raw(p), Qty::from_raw(q)))
            .collect()
    }

    /// Top `n` asks, lowest price first.
    #[must_use]
    pub fn asks_top(&self, n: usize) -> Vec<(Price, Qty)> {
        self.asks
            .iter()
            .take(n)
            .map(|(&p, &q)| (Price::from_raw(p), Qty::from_raw(q)))
            .collect()
    }

    /// Iterate bids best-first (highest price first) without allocating.
    pub fn bids_iter(&self) -> impl DoubleEndedIterator<Item = (Price, Qty)> + '_ {
        self.bids
            .iter()
            .rev()
            .map(|(&p, &q)| (Price::from_raw(p), Qty::from_raw(q)))
    }

    /// Iterate asks best-first (lowest price first) without allocating.
    pub fn asks_iter(&self) -> impl DoubleEndedIterator<Item = (Price, Qty)> + '_ {
        self.asks
            .iter()
            .map(|(&p, &q)| (Price::from_raw(p), Qty::from_raw(q)))
    }

    /// Resting quantity at an exact price on `side`, if any. Used to measure the
    /// queue ahead of a new maker order.
    #[must_use]
    pub fn qty_at(&self, side: Side, price: Price) -> Option<Qty> {
        let book = match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        book.get(&price.raw()).map(|&q| Qty::from_raw(q))
    }

    /// Number of bid price levels.
    #[must_use]
    pub fn bid_levels(&self) -> usize {
        self.bids.len()
    }

    /// Number of ask price levels.
    #[must_use]
    pub fn ask_levels(&self) -> usize {
        self.asks.len()
    }

    /// Count of level mutations applied so far.
    #[must_use]
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Count of crossed levels trimmed (a feed-health / warm-up indicator).
    #[must_use]
    pub fn crossed_trims(&self) -> u64 {
        self.crossed_trims
    }

    /// Count of far-from-touch levels evicted by the depth cap.
    #[must_use]
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Configured max levels per side (0 = unlimited).
    #[must_use]
    pub fn max_levels(&self) -> usize {
        self.max_levels
    }

    /// `local_ts` of the most recent event applied.
    #[must_use]
    pub fn last_ts(&self) -> u64 {
        self.last_ts
    }

    /// True only if the top of book is crossed (should never happen post-trim).
    #[must_use]
    pub fn is_crossed(&self) -> bool {
        match (self.bids.iter().next_back(), self.asks.iter().next()) {
            (Some((&b, _)), Some((&a, _))) => b >= a,
            _ => false,
        }
    }

    /// Deterministic FNV-1a hash of the full book state (both sides, in order).
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = FNV_OFFSET;
        for (&p, &q) in &self.bids {
            fold_i64(&mut h, p);
            fold_i64(&mut h, q);
        }
        fold_i64(&mut h, i64::MIN); // side separator
        for (&p, &q) in &self.asks {
            fold_i64(&mut h, p);
            fold_i64(&mut h, q);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{EventKind, Price, Qty, Side, UnixNanos};

    fn delta(side: Side, px: i64, qty: i64, ts: u64) -> Event {
        Event::new(
            EventKind::BookDelta,
            UnixNanos::new(ts),
            UnixNanos::new(ts),
            Some(side),
            Price::from_raw(px),
            Qty::from_raw(qty),
            0,
        )
        .unwrap()
    }

    #[test]
    fn applies_and_reads_best() {
        let mut b = OrderBook::new();
        b.apply(&delta(Side::Bid, 100, 5, 1)).unwrap();
        b.apply(&delta(Side::Bid, 99, 7, 2)).unwrap();
        b.apply(&delta(Side::Ask, 101, 4, 3)).unwrap();
        b.apply(&delta(Side::Ask, 102, 9, 4)).unwrap();
        assert_eq!(b.best_bid(), Some((Price::from_raw(100), Qty::from_raw(5))));
        assert_eq!(b.best_ask(), Some((Price::from_raw(101), Qty::from_raw(4))));
        assert_eq!(b.spread(), Some(Price::from_raw(1)));
        assert_eq!(b.bid_levels(), 2);
        assert_eq!(b.ask_levels(), 2);
        assert!(!b.is_crossed());
    }

    #[test]
    fn remove_deletes_level() {
        let mut b = OrderBook::new();
        b.apply(&delta(Side::Bid, 100, 5, 1)).unwrap();
        b.apply(&delta(Side::Bid, 100, 0, 2)).unwrap();
        assert_eq!(b.best_bid(), None);
        assert_eq!(b.bid_levels(), 0);
    }

    #[test]
    fn crossing_bid_trims_asks() {
        let mut b = OrderBook::new();
        b.apply(&delta(Side::Ask, 101, 4, 1)).unwrap();
        b.apply(&delta(Side::Ask, 102, 9, 2)).unwrap();
        b.apply(&delta(Side::Bid, 101, 3, 3)).unwrap();
        assert!(!b.is_crossed());
        assert_eq!(b.best_ask(), Some((Price::from_raw(102), Qty::from_raw(9))));
        assert_eq!(b.best_bid(), Some((Price::from_raw(101), Qty::from_raw(3))));
        assert_eq!(b.crossed_trims(), 1);
    }

    #[test]
    fn missing_side_fails_fast() {
        let ev = Event::new(
            EventKind::BookDelta,
            UnixNanos::new(1),
            UnixNanos::new(1),
            None,
            Price::from_raw(100),
            Qty::from_raw(5),
            0,
        )
        .unwrap();
        let mut b = OrderBook::new();
        assert!(b.apply(&ev).is_err());
    }

    #[test]
    fn trade_and_quote_do_not_mutate() {
        let mut b = OrderBook::new();
        b.apply(&delta(Side::Bid, 100, 5, 1)).unwrap();
        let before = b.state_hash();
        let trade = Event::new(
            EventKind::Trade,
            UnixNanos::new(2),
            UnixNanos::new(2),
            Some(Side::Ask),
            Price::from_raw(100),
            Qty::from_raw(1),
            0,
        )
        .unwrap();
        b.apply(&trade).unwrap();
        assert_eq!(b.state_hash(), before);
    }

    #[test]
    fn fold_is_deterministic() {
        let evs = [
            delta(Side::Bid, 100, 5, 1),
            delta(Side::Ask, 101, 4, 2),
            delta(Side::Bid, 99, 7, 3),
            delta(Side::Ask, 101, 6, 4),
            delta(Side::Bid, 100, 0, 5),
        ];
        let mut a = OrderBook::new();
        let mut c = OrderBook::new();
        for e in &evs {
            a.apply(e).unwrap();
        }
        for e in &evs {
            c.apply(e).unwrap();
        }
        assert_eq!(a.state_hash(), c.state_hash());
    }

    #[test]
    fn depth_cap_keeps_near_touch_and_preserves_top() {
        let mut full = OrderBook::new();
        let mut capped = OrderBook::with_max_levels(3);
        let evs = [
            delta(Side::Bid, 100, 1, 1),
            delta(Side::Bid, 99, 1, 2),
            delta(Side::Bid, 98, 1, 3),
            delta(Side::Bid, 97, 1, 4),
            delta(Side::Bid, 96, 1, 5),
            delta(Side::Ask, 101, 1, 6),
            delta(Side::Ask, 102, 1, 7),
            delta(Side::Ask, 103, 1, 8),
            delta(Side::Ask, 104, 1, 9),
            delta(Side::Ask, 105, 1, 10),
        ];
        for e in &evs {
            full.apply(e).unwrap();
            capped.apply(e).unwrap();
        }
        assert_eq!(full.best_bid(), capped.best_bid());
        assert_eq!(full.best_ask(), capped.best_ask());
        assert_eq!(capped.bids_top(3), full.bids_top(3));
        assert_eq!(capped.asks_top(3), full.asks_top(3));
        assert_eq!(capped.bid_levels(), 3);
        assert_eq!(capped.ask_levels(), 3);
        assert_eq!(capped.evicted(), 4);
    }
}