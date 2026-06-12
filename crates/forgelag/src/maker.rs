//! Maker components for the lagshot-maker pivot (resting-limit capture of the
//! basis-reversion edge). Kept in their own module so the taker `Lagshot`
//! machinery in `strategy.rs` stays clean. This file currently houses the
//! [`FairValueOracle`] (Task 4); the InventoryController, QuoteManager, and
//! MakerQuoter land here in later tasks.

use crate::engine::LagCtx;

/// Default reference-staleness limit: 1000 ms on the virtual clock (Req 2.5).
pub const DEFAULT_STALENESS_NS: u64 = 1_000_000_000;

/// Min gap samples before the deviation is trusted (mirrors `BasisSignal`'s
/// `ring.len() >= 20` warm-up so early noise does not fire).
const MIN_DEV_SAMPLES: usize = 20;

/// Configuration for the [`FairValueOracle`].
#[derive(Clone, Copy, Debug)]
pub struct FairValueConfig {
    /// Microprice depth (levels per side), matching `BasisSignal::micro`.
    pub top_n: usize,
    /// Rolling baseline length (gap samples).
    pub window: usize,
    /// Gap-sample cadence (ns).
    pub sample_ns: u64,
    /// Reference staleness limit (ns); default [`DEFAULT_STALENESS_NS`].
    pub staleness_ns: u64,
}
impl Default for FairValueConfig {
    fn default() -> Self {
        Self { top_n: 5, window: 500, sample_ns: 500_000_000, staleness_ns: DEFAULT_STALENESS_NS }
    }
}

/// Computes where HL is expected to trade next: the latest valid OKX reference
/// price adjusted by the rolling basis baseline (the gap HL reverts toward).
///
/// Reuses `BasisSignal`'s gap/baseline sampling math exactly:
/// `gap_bps = (micro - ref_px)/ref_px * 1e4`, `baseline` = rolling mean of the
/// gap over `window` samples, `dev = gap - baseline`. Fair value is then
/// `ref_px * (1 + baseline_bps/1e4)`.
///
/// All updates use only data with `ts <= now` (no lookahead, Req 2.2/9.3): the
/// engine's `LagCtx` only ever exposes `<= now` state, and this struct never
/// looks past what `observe`/`update` is handed.
pub struct FairValueOracle {
    cfg: FairValueConfig,
    // rolling gap-sample ring (mean = baseline).
    ring: Vec<f64>,
    pos: usize,
    sum: f64,
    // sampling cadence bookkeeping.
    next_sample: u64,
    started: bool,
    // latest reference price + the virtual-clock time it last changed.
    ref_px: f64,
    last_ref_ts: u64,
    have_ref: bool,
    // current gap / deviation state.
    cur_gap: f64,
    have_gap: bool,
    cur_dev: f64,
    have_dev: bool,
}

impl FairValueOracle {
    /// Build an oracle. `top_n`, `window`, and `sample_ns` are floored at 1;
    /// `staleness_ns` of 0 is replaced by the default (a 0 limit would make
    /// every read stale and is never intended).
    #[must_use]
    pub fn new(cfg: FairValueConfig) -> Self {
        let staleness_ns = if cfg.staleness_ns == 0 { DEFAULT_STALENESS_NS } else { cfg.staleness_ns };
        Self {
            cfg: FairValueConfig {
                top_n: cfg.top_n.max(1),
                window: cfg.window.max(1),
                sample_ns: cfg.sample_ns.max(1),
                staleness_ns,
            },
            ring: Vec::new(),
            pos: 0,
            sum: 0.0,
            next_sample: 0,
            started: false,
            ref_px: 0.0,
            last_ref_ts: 0,
            have_ref: false,
            cur_gap: 0.0,
            have_gap: false,
            cur_dev: 0.0,
            have_dev: false,
        }
    }

    /// Size-weighted top-N microprice from the execution book. Copied verbatim
    /// from `BasisSignal::micro` so the oracle and the taker signal agree:
    /// `(bb*aq + ba*bq)/(aq+bq)`, falling back to the mid when there is no depth.
    fn micro(&self, ctx: &LagCtx) -> Option<f64> {
        let (bb, _) = ctx.exec_book.best_bid()?;
        let (ba, _) = ctx.exec_book.best_ask()?;
        let bb = bb.to_f64();
        let ba = ba.to_f64();
        let bq: f64 = ctx.exec_book.bids_iter().take(self.cfg.top_n).map(|(_, q)| q.raw() as f64).sum();
        let aq: f64 = ctx.exec_book.asks_iter().take(self.cfg.top_n).map(|(_, q)| q.raw() as f64).sum();
        let tot = bq + aq;
        if tot <= 0.0 {
            return Some((bb + ba) / 2.0);
        }
        Some((bb * aq + ba * bq) / tot)
    }

    /// Push a gap sample into the rolling ring (mean tracked incrementally),
    /// mirroring `BasisSignal::push_sample`.
    fn push_sample(&mut self, g: f64) {
        if self.ring.len() < self.cfg.window {
            self.ring.push(g);
            self.sum += g;
        } else {
            let old = self.ring[self.pos];
            self.sum += g - old;
            self.ring[self.pos] = g;
            self.pos = (self.pos + 1) % self.cfg.window;
        }
    }

    /// Current rolling baseline (mean gap) in bps. 0.0 before the first sample.
    #[must_use]
    pub fn baseline_bps(&self) -> f64 {
        if self.ring.is_empty() {
            0.0
        } else {
            self.sum / self.ring.len() as f64
        }
    }

    /// Lower-level update used by [`FairValueOracle::observe`] and by unit tests
    /// (drives the math without needing a full `OrderBook`). Tracks the latest
    /// reference price and the time it last changed, then samples the gap on the
    /// configured cadence. `now` is the virtual clock; all inputs are `<= now`.
    fn update(&mut self, now: u64, ref_px: f64, micro: Option<f64>) {
        if ref_px > 0.0 {
            // A new reference price refreshes the staleness clock (Req 2.5).
            if !self.have_ref || ref_px != self.ref_px {
                self.last_ref_ts = now;
            }
            self.ref_px = ref_px;
            self.have_ref = true;
            if let Some(m) = micro {
                self.cur_gap = (m - ref_px) / ref_px * 10_000.0;
                self.have_gap = true;
            }
        }
        if self.have_gap {
            if !self.started {
                self.next_sample = now;
                self.started = true;
            }
            if now >= self.next_sample {
                // dev = current gap minus the baseline BEFORE this sample joins,
                // identical to BasisSignal::cur_dev.
                let base = if self.ring.is_empty() { self.cur_gap } else { self.sum / self.ring.len() as f64 };
                self.cur_dev = self.cur_gap - base;
                self.have_dev = self.ring.len() >= MIN_DEV_SAMPLES;
                self.push_sample(self.cur_gap);
                while self.next_sample <= now {
                    self.next_sample += self.cfg.sample_ns;
                }
            }
        }
    }

    /// Called each event: read the reference price + microprice from the
    /// `<= now` context and update the rolling baseline / deviation.
    pub fn observe(&mut self, ctx: &LagCtx) {
        let micro = self.micro(ctx);
        self.update(ctx.now, ctx.ref_px, micro);
    }

    /// Fair value for HL = `ref_px * (1 + baseline_bps/1e4)`, the price HL is
    /// expected to revert toward. `None` until the first reference price (Req
    /// 2.1, 2.4).
    #[must_use]
    pub fn fair_value(&self) -> Option<f64> {
        if !self.have_ref {
            return None;
        }
        Some(self.ref_px * (1.0 + self.baseline_bps() / 10_000.0))
    }

    /// Current deviation (dislocation signal) in bps = `gap - baseline`, the
    /// same quantity as `BasisSignal::cur_dev`. `None` until enough samples have
    /// warmed up so early noise does not fire.
    #[must_use]
    pub fn dev_bps(&self) -> Option<f64> {
        if self.have_dev {
            Some(self.cur_dev)
        } else {
            None
        }
    }

    /// True once enough gap samples have accumulated for the deviation to be
    /// trusted (mirrors `BasisSignal`'s warm-up gate).
    #[must_use]
    pub fn ready(&self) -> bool {
        self.have_dev
    }

    /// Whether the latest reference price is older than the staleness limit
    /// (Req 2.5). Also stale when no reference has been observed yet (no data
    /// to quote around). Uses `now - last_ref_ts > staleness_ns` (strict).
    #[must_use]
    pub fn is_stale(&self, now: u64) -> bool {
        if !self.have_ref {
            return true;
        }
        now.saturating_sub(self.last_ref_ts) > self.cfg.staleness_ns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FairValueConfig {
        // small cadence so each update() samples; window large, irrelevant here.
        FairValueConfig { top_n: 5, window: 500, sample_ns: 1, staleness_ns: DEFAULT_STALENESS_NS }
    }

    #[test]
    fn fair_value_none_before_any_reference() {
        let o = FairValueOracle::new(cfg());
        assert_eq!(o.fair_value(), None, "no ref yet => no fair value (Req 2.4)");
        assert_eq!(o.dev_bps(), None);
        assert!(!o.ready());
        // stale when no reference has ever been seen.
        assert!(o.is_stale(0));
        assert!(o.is_stale(10_000));
    }

    #[test]
    fn fair_value_equals_ref_when_baseline_zero() {
        let mut o = FairValueOracle::new(cfg());
        // micro == ref => gap 0 => baseline 0 => fair value == ref.
        o.update(100, 2000.0, Some(2000.0));
        let fv = o.fair_value().expect("have ref now");
        assert!((fv - 2000.0).abs() < 1e-6, "fv {fv} should equal ref when baseline 0");
    }

    #[test]
    fn baseline_and_dev_compute_correctly() {
        let mut o = FairValueOracle::new(cfg());
        // Feed a constant +10 bps gap (micro 0.1% above ref) many times.
        // ref 1000, micro 1001 => gap = (1001-1000)/1000*1e4 = 10 bps.
        for t in 0..60u64 {
            o.update(t, 1000.0, Some(1001.0));
        }
        // baseline should converge to the constant gap (~10 bps).
        let base = o.baseline_bps();
        assert!((base - 10.0).abs() < 1e-6, "baseline {base} should be ~10 bps");
        // dev = current gap - baseline; with a constant gap it collapses to ~0.
        assert!(o.ready(), "should be warmed up after 60 samples");
        let dev = o.dev_bps().expect("warm");
        assert!(dev.abs() < 1e-6, "dev {dev} should be ~0 for a constant gap");
        // fair value = ref * (1 + baseline/1e4) = 1000 * (1 + 10/1e4) = 1001.
        let fv = o.fair_value().expect("have ref");
        assert!((fv - 1001.0).abs() < 1e-6, "fv {fv} should equal the microprice the gap implies");
    }

    #[test]
    fn dev_reacts_to_a_fresh_dislocation() {
        let mut o = FairValueOracle::new(cfg());
        // Warm up with a flat 0-gap baseline.
        for t in 0..40u64 {
            o.update(t, 1000.0, Some(1000.0));
        }
        assert!(o.ready());
        assert!(o.dev_bps().expect("warm").abs() < 1e-6);
        // Now HL jumps rich: micro 1002 => gap = 20 bps vs ~0 baseline.
        o.update(41, 1000.0, Some(1002.0));
        let dev = o.dev_bps().expect("warm");
        assert!(dev > 15.0, "dev {dev} should spike positive on a fresh rich dislocation");
    }

    #[test]
    fn warmup_gate_holds_dev_until_enough_samples() {
        let mut o = FairValueOracle::new(cfg());
        // Fewer than MIN_DEV_SAMPLES updates => dev not trusted yet.
        for t in 0..(MIN_DEV_SAMPLES as u64 - 1) {
            o.update(t, 1000.0, Some(1001.0));
        }
        assert!(!o.ready(), "should not be warm before {MIN_DEV_SAMPLES} samples");
        assert_eq!(o.dev_bps(), None);
        // fair value is available as soon as we have a ref, though.
        assert!(o.fair_value().is_some());
    }

    #[test]
    fn staleness_flips_after_limit_and_resets_on_update() {
        let mut o = FairValueOracle::new(cfg());
        let t0 = 5_000_000_000u64;
        o.update(t0, 1000.0, Some(1000.0));
        // Right after an update: fresh.
        assert!(!o.is_stale(t0));
        // At exactly the limit: not yet stale (strict >).
        assert!(!o.is_stale(t0 + DEFAULT_STALENESS_NS));
        // One ns past the limit with no ref update: stale.
        assert!(o.is_stale(t0 + DEFAULT_STALENESS_NS + 1));
        // A fresh reference price resets the staleness clock.
        let t1 = t0 + DEFAULT_STALENESS_NS + 1;
        o.update(t1, 1000.5, Some(1000.5));
        assert!(!o.is_stale(t1));
        assert!(!o.is_stale(t1 + DEFAULT_STALENESS_NS));
        assert!(o.is_stale(t1 + DEFAULT_STALENESS_NS + 1));
    }

    #[test]
    fn unchanged_reference_does_not_refresh_staleness_clock() {
        let mut o = FairValueOracle::new(cfg());
        let t0 = 1_000_000_000u64;
        o.update(t0, 1000.0, Some(1000.0));
        // Same ref value arriving later must NOT reset last_ref_ts: a frozen
        // feed repeating the same price should still be detected as stale.
        let t_repeat = t0 + DEFAULT_STALENESS_NS + 1;
        o.update(t_repeat, 1000.0, Some(1000.0));
        assert!(o.is_stale(t_repeat), "repeat of the same ref price must not look fresh");
    }

    #[test]
    fn frozen_feed_retains_last_fair_value() {
        let mut o = FairValueOracle::new(cfg());
        for t in 0..30u64 {
            o.update(t, 1000.0, Some(1001.0));
        }
        let fv_before = o.fair_value().expect("have fv");
        // No further updates (feed frozen). Reading later does not change fv.
        assert!(o.is_stale(10_000_000_000));
        let fv_after = o.fair_value().expect("still have last fv (Req 2.5)");
        assert!((fv_before - fv_after).abs() < 1e-12, "frozen feed must retain last fair value");
    }
}

// ===========================================================================
// InventoryController (Task 5, Req 4) - signed position cap + inventory skew.
// Advisory only: it reads the exchange-truth signed position from
// `ctx.position_qty` and ADVISES the quote layer. It never mutates position
// (the Account does that); the QuoteManager (Task 6) consumes its verdicts.
// ===========================================================================

use forge_core::Side;

/// Configuration for the [`InventoryController`].
#[derive(Clone, Copy, Debug)]
pub struct InventoryConfig {
    /// Hard cap on the absolute signed position, in raw base quantity (Req 4.2).
    /// A new quote is suppressed if it would push `|position|` STRICTLY above
    /// this. Valid range: `>= 0` (Req 4.5).
    pub pos_cap: i64,
    /// Inventory skew strength in bps at full cap (Req 4.3). `>= 0` (Req 4.5).
    /// The skew offset scales as `inv_skew_bps * |position| / pos_cap`, so at
    /// `|position| == pos_cap` the offset magnitude equals `inv_skew_bps`.
    pub inv_skew_bps: f64,
    /// Size of one quote, in raw base quantity. Needed for the cap check
    /// (resulting position) and the "cap smaller than one quote size" rule
    /// (Req 4.7).
    pub quote_qty: i64,
}
impl Default for InventoryConfig {
    fn default() -> Self {
        Self { pos_cap: 0, inv_skew_bps: 0.0, quote_qty: 1 }
    }
}

/// Enforces the position cap and computes the inventory skew offset.
///
/// # Skew sign convention (READ THIS - Task 6 must apply it correctly)
/// [`InventoryController::skew_bps`] returns a SIGNED bps offset that the
/// QuoteManager ADDS to the base `quote_offset_bps` (a non-negative DISTANCE
/// from fair value) for the given side:
///
/// ```text
///   effective_offset_bps = base_quote_offset_bps + skew_bps(position, side)
/// ```
///
/// - A NEGATIVE return moves the quote CLOSER to mid (smaller distance from
///   fair value => easier fill). This is what the position-REDUCING side gets.
/// - A POSITIVE return moves the quote FURTHER from mid (larger distance =>
///   harder fill). This is what the position-INCREASING side gets.
/// - At flat (`position == 0`) both sides return `0.0` (symmetric, Req 4.4).
///
/// "Reducing" = the side whose fill shrinks `|position|`: while LONG
/// (`position > 0`) that is the Ask (a sell), while SHORT it is the Bid.
pub struct InventoryController {
    cfg: InventoryConfig,
}

impl InventoryController {
    /// Build a controller. `pos_cap` is floored at 0 and `inv_skew_bps` at 0.0
    /// (their valid ranges, Req 4.5); `quote_qty` is floored at 1 (a quote
    /// always has positive size).
    #[must_use]
    pub fn new(cfg: InventoryConfig) -> Self {
        Self {
            cfg: InventoryConfig {
                pos_cap: cfg.pos_cap.max(0),
                inv_skew_bps: cfg.inv_skew_bps.max(0.0),
                quote_qty: cfg.quote_qty.max(1),
            },
        }
    }

    /// True if the cap is too small to ever rest a single quote, i.e. the
    /// configured `pos_cap` is smaller than one `quote_qty` (Req 4.7). When
    /// true, [`InventoryController::allows`] suppresses every quote for the run.
    #[must_use]
    pub fn cap_too_small(&self) -> bool {
        self.cfg.pos_cap < self.cfg.quote_qty
    }

    /// Signed position after a fill on `side` of one `quote_qty`:
    /// a Bid (buy) increases the position, an Ask (sell) decreases it (Req 4.1).
    fn resulting_position(&self, position: i64, side: Side) -> i64 {
        match side {
            Side::Bid => position.saturating_add(self.cfg.quote_qty),
            Side::Ask => position.saturating_sub(self.cfg.quote_qty),
        }
    }

    /// Whether a new quote on `side` is allowed given the current signed
    /// `position`. Returns `false` (suppress the quote) when:
    /// - the cap is smaller than one quote size (suppress all, Req 4.7), or
    /// - resting this quote would push `|resulting position|` STRICTLY above
    ///   `pos_cap` (Req 4.2).
    ///
    /// Otherwise `true`. The position-reducing side near the cap stays allowed
    /// because it shrinks `|position|`.
    #[must_use]
    pub fn allows(&self, position: i64, side: Side) -> bool {
        if self.cap_too_small() {
            return false;
        }
        self.resulting_position(position, side).unsigned_abs() <= self.cfg.pos_cap.unsigned_abs()
    }

    /// Signed inventory-skew offset in bps for `side` at the current signed
    /// `position`. See the type-level docs for the exact sign convention:
    /// negative = closer to mid (reducing side), positive = further (increasing
    /// side), `0.0` at flat. Magnitude grows monotonically with `|position|`
    /// as `inv_skew_bps * |position| / pos_cap` (Req 4.3).
    #[must_use]
    pub fn skew_bps(&self, position: i64, side: Side) -> f64 {
        if position == 0 || self.cfg.pos_cap <= 0 || self.cfg.inv_skew_bps == 0.0 {
            return 0.0;
        }
        let frac = position.unsigned_abs() as f64 / self.cfg.pos_cap.unsigned_abs() as f64;
        let mag = self.cfg.inv_skew_bps * frac;
        // Reducing side: long => Ask reduces; short => Bid reduces.
        let reducing = (position > 0 && side == Side::Ask) || (position < 0 && side == Side::Bid);
        if reducing {
            -mag
        } else {
            mag
        }
    }
}

#[cfg(test)]
mod inv_tests {
    use super::*;

    fn ctrl(pos_cap: i64, inv_skew_bps: f64, quote_qty: i64) -> InventoryController {
        InventoryController::new(InventoryConfig { pos_cap, inv_skew_bps, quote_qty })
    }

    #[test]
    fn flat_allows_both_sides() {
        // cap comfortably larger than one quote: at flat either side is fine.
        let c = ctrl(100, 0.0, 10);
        assert!(c.allows(0, Side::Bid), "flat bid allowed (Req 4.2)");
        assert!(c.allows(0, Side::Ask), "flat ask allowed (Req 4.2)");
    }

    #[test]
    fn at_positive_cap_suppresses_increasing_allows_reducing() {
        // Long at the cap: a further Bid (increasing) would breach; an Ask
        // (reducing) shrinks |pos| and stays allowed (Req 4.2).
        let c = ctrl(100, 0.0, 10);
        assert!(!c.allows(100, Side::Bid), "bid at +cap pushes |pos| 110 > 100 => suppress");
        assert!(c.allows(100, Side::Ask), "ask at +cap reduces to 90 => allowed");
        // Boundary: resulting exactly == cap is allowed (only STRICTLY above is suppressed).
        assert!(c.allows(90, Side::Bid), "bid to exactly +cap (100) is allowed");
        assert!(!c.allows(91, Side::Bid), "bid to 101 (> cap) is suppressed");
    }

    #[test]
    fn at_negative_cap_suppresses_increasing_allows_reducing() {
        // Symmetric for short at -cap.
        let c = ctrl(100, 0.0, 10);
        assert!(!c.allows(-100, Side::Ask), "ask at -cap pushes |pos| 110 > 100 => suppress");
        assert!(c.allows(-100, Side::Bid), "bid at -cap reduces to -90 => allowed");
    }

    #[test]
    fn cap_smaller_than_quote_suppresses_all_quotes_always() {
        // Req 4.7: pos_cap < one quote size => suppress ALL quotes, any state.
        let c = ctrl(5, 1.0, 10);
        assert!(c.cap_too_small());
        for pos in [-20, -5, 0, 5, 20] {
            assert!(!c.allows(pos, Side::Bid), "cap<quote: bid suppressed at pos {pos}");
            assert!(!c.allows(pos, Side::Ask), "cap<quote: ask suppressed at pos {pos}");
        }
    }

    #[test]
    fn skew_zero_at_flat() {
        // Req 4.4: symmetric, no skew at zero inventory.
        let c = ctrl(100, 5.0, 10);
        assert_eq!(c.skew_bps(0, Side::Bid), 0.0);
        assert_eq!(c.skew_bps(0, Side::Ask), 0.0);
    }

    #[test]
    fn skew_leans_reducing_side_closer_to_mid_when_long() {
        // Long inventory: Ask is reducing => closer to mid (negative offset);
        // Bid is increasing => further (positive). Reducing < increasing.
        let c = ctrl(100, 2.0, 10);
        let ask = c.skew_bps(50, Side::Ask);
        let bid = c.skew_bps(50, Side::Bid);
        assert!(ask < 0.0, "long: reducing Ask offset negative (closer to mid), got {ask}");
        assert!(bid > 0.0, "long: increasing Bid offset positive (further), got {bid}");
        assert!(ask < bid, "reducing side rests closer to mid than increasing side");
        // sign convention: equal magnitude, opposite sign at a given |pos|.
        assert!((ask + bid).abs() < 1e-12, "symmetric magnitude about the base offset");
    }

    #[test]
    fn skew_leans_reducing_side_closer_to_mid_when_short() {
        // Short inventory: Bid reduces => negative; Ask increases => positive.
        let c = ctrl(100, 2.0, 10);
        let bid = c.skew_bps(-50, Side::Bid);
        let ask = c.skew_bps(-50, Side::Ask);
        assert!(bid < 0.0, "short: reducing Bid offset negative (closer to mid), got {bid}");
        assert!(ask > 0.0, "short: increasing Ask offset positive (further), got {ask}");
        assert!(bid < ask, "reducing side rests closer to mid than increasing side");
    }

    #[test]
    fn skew_magnitude_grows_monotonically_with_abs_position() {
        // Req 4.3: offset grows monotonically with |position|.
        let c = ctrl(100, 2.0, 10);
        let s25 = c.skew_bps(25, Side::Ask).abs();
        let s50 = c.skew_bps(50, Side::Ask).abs();
        let s100 = c.skew_bps(100, Side::Ask).abs();
        assert!(s25 < s50, "|skew| grows from 25 -> 50: {s25} < {s50}");
        assert!(s50 < s100, "|skew| grows from 50 -> 100: {s50} < {s100}");
        // at full cap the magnitude equals inv_skew_bps.
        assert!((s100 - 2.0).abs() < 1e-12, "at |pos|==cap the skew == inv_skew_bps");
    }

    #[test]
    fn zero_skew_config_means_no_skew() {
        // inv_skew_bps = 0 => no skew regardless of position (Req 4.5 boundary).
        let c = ctrl(100, 0.0, 10);
        assert_eq!(c.skew_bps(80, Side::Ask), 0.0);
        assert_eq!(c.skew_bps(-80, Side::Bid), 0.0);
    }
}
// ===========================================================================
// QuoteManager (Task 6, Req 1.2/1.3/1.4/2.3/3.1/3.2/3.3/3.5/3.7)
// ---------------------------------------------------------------------------
// Owns the single resting ENTRY quote's lifecycle WHILE FLAT, plus the
// danger-pull / emergency-flatten watchdog. It is a reusable component the
// Task-7 `MakerQuoter` strategy drives; it is NOT itself a `LagStrategy`.
//
// SPLIT between QuoteManager and the future MakerQuoter:
//   - QuoteManager owns: WHERE to rest (offset from fair value on the reversion
//     side + inventory skew), WHEN to keep / reprice / pull the single entry
//     quote, and the danger-pull watchdog that surfaces an emergency flatten.
//   - MakerQuoter (Task 7) owns: the position/exit half - taker revert-to-mean
//     exit, fee wiring, and the overall FLAT/IN-POSITION state machine. It will
//     call `observe` every event, `on_event`/`manage_entry` while FLAT, and
//     `take_emergency_flatten` while IN POSITION.
//
// EMERGENCY FLATTEN is surfaced BOTH ways: `take_emergency_flatten` returns the
// Market order for Task 7 to emit from its in-position branch, and `on_event`
// (the standalone driver used by the unit tests) emits it directly with top
// priority. Either path clears the watchdog so it fires exactly once.
//
// CONTRACT: the caller MUST call `observe(ctx)` exactly once per event BEFORE
// `on_event`/`manage_entry` so the oracle samples on its cadence without being
// double-counted. `on_event` deliberately does NOT observe.
// ===========================================================================

use crate::engine::LagOrder;
use forge_core::{Price, Qty};

/// Configuration for the [`QuoteManager`]. All bps fields are interpreted the
/// same way as the oracle/inventory (basis points of fair value).
#[derive(Clone, Copy, Debug)]
pub struct QuoteConfig {
    /// Base distance from fair value to rest the quote, in bps (Req 2.3). The
    /// signed inventory skew is ADDED to this before placement.
    pub quote_offset_bps: f64,
    /// `|dev|` (bps) required to WANT an entry quote (Req 1.2). Entries fire in
    /// the band `[entry_threshold_bps, danger_bps)`.
    pub entry_threshold_bps: f64,
    /// `|dev|` (bps) at/below which a resting entry quote is cancelled because
    /// the dislocation reverted (Req 1.4).
    pub exit_bps: f64,
    /// Keep-vs-reprice band in bps of fair value (Req 3.1/3.2). The quote is
    /// kept while the target stays within this of the resting price.
    pub reprice_tol_bps: f64,
    /// `|dev|` (bps) at/above which the basis has WIDENED into a run-over and
    /// the quote is pulled (Req 3.3). Also caps the entry band from above.
    pub danger_bps: f64,
    /// Cancel/reprice latency (ns); used to time the emergency-flatten watchdog.
    pub cancel_latency_ns: u64,
    /// Acknowledgement timeout (ns) added after the cancel latency; if the pull
    /// has not resolved within `cancel_latency_ns + ack_timeout_ns` and the
    /// position is non-zero, an emergency flatten fires (Req 3.7).
    pub ack_timeout_ns: u64,
    /// Size of one quote (raw base qty); matches the inventory controller's.
    pub quote_qty: Qty,
}
impl Default for QuoteConfig {
    fn default() -> Self {
        Self {
            quote_offset_bps: 2.0,
            entry_threshold_bps: 16.0,
            exit_bps: 2.0,
            reprice_tol_bps: 1.0,
            danger_bps: 40.0,
            cancel_latency_ns: DEFAULT_STALENESS_NS,
            ack_timeout_ns: DEFAULT_STALENESS_NS,
            quote_qty: Qty::from_raw(1),
        }
    }
}

/// The single live entry quote we are tracking.
#[derive(Clone, Copy, Debug)]
struct RestingQuote {
    /// Client id assigned at placement (addresses the order for cancel).
    id: u64,
    /// Our resting side (Bid below fair value, Ask above).
    side: Side,
    /// Raw fixed-point resting price.
    price_raw: i64,
}

/// A pull issued while in danger; arms the emergency-flatten watchdog.
#[derive(Clone, Copy, Debug)]
struct DangerPull {
    /// Virtual-clock time by which the cancel must have resolved; past this with
    /// a non-zero position we emergency-flatten (Req 3.7).
    deadline: u64,
}

/// Decides the entry-quote lifecycle: target price (offset from fair value on
/// the reversion side + inventory skew), keep / reprice / pull, and the
/// emergency flatten when a danger-pull fails to land before a fill.
pub struct QuoteManager {
    cfg: QuoteConfig,
    oracle: FairValueOracle,
    inv: InventoryController,
    resting: Option<RestingQuote>,
    danger_pull: Option<DangerPull>,
    next_id: u64,
}

impl QuoteManager {
    /// Build a quote manager around an oracle and an inventory controller.
    /// Client ids start at 1 (0 is reserved for taker/legacy orders).
    #[must_use]
    pub fn new(cfg: QuoteConfig, oracle: FairValueOracle, inv: InventoryController) -> Self {
        Self { cfg, oracle, inv, resting: None, danger_pull: None, next_id: 1 }
    }

    /// Sample the oracle for this event. MUST be called once per event before
    /// [`QuoteManager::on_event`] / [`QuoteManager::manage_entry`].
    pub fn observe(&mut self, ctx: &LagCtx) {
        self.oracle.observe(ctx);
    }

    /// Fair value passthrough (for Task 7 / metrics).
    #[must_use]
    pub fn fair_value(&self) -> Option<f64> {
        self.oracle.fair_value()
    }

    /// Deviation passthrough (for Task 7 / metrics).
    #[must_use]
    pub fn dev_bps(&self) -> Option<f64> {
        self.oracle.dev_bps()
    }

    /// Staleness passthrough (for Task 7).
    #[must_use]
    pub fn is_stale(&self, now: u64) -> bool {
        self.oracle.is_stale(now)
    }

    /// Whether an entry quote is currently tracked as resting.
    #[must_use]
    pub fn has_resting(&self) -> bool {
        self.resting.is_some()
    }

    /// Reversion side from the deviation sign: `dev > 0` (HL rich) -> rest an
    /// Ask above fair value; `dev < 0` (HL cheap) -> rest a Bid below it. `None`
    /// when `dev == 0` (no dislocation, no side).
    fn reversion_side(dev: f64) -> Option<Side> {
        if dev > 0.0 {
            Some(Side::Ask)
        } else if dev < 0.0 {
            Some(Side::Bid)
        } else {
            None
        }
    }

    /// Effective offset (bps) from fair value for `side` at `position`:
    /// `base quote_offset_bps + signed inventory skew`, floored at 0 so the
    /// quote never crosses to the wrong side of fair value (Req 2.3, 4.3).
    fn effective_offset_bps(&self, side: Side, position: i64) -> f64 {
        (self.cfg.quote_offset_bps + self.inv.skew_bps(position, side)).max(0.0)
    }

    /// Target resting price: an Ask rests ABOVE fair value at `fv*(1+off/1e4)`,
    /// a Bid BELOW at `fv*(1-off/1e4)`, where `off` is the effective offset.
    /// `None` only if the price is not representable (never for sane inputs).
    fn target_price(&self, fv: f64, side: Side, position: i64) -> Option<Price> {
        let off = self.effective_offset_bps(side, position) / 10_000.0;
        let px = match side {
            Side::Ask => fv * (1.0 + off),
            Side::Bid => fv * (1.0 - off),
        };
        Price::from_f64(px).ok()
    }

    /// Emit a cancel for the tracked resting quote (if any) and stop tracking
    /// it. The order remains fillable in the engine until the cancel lands
    /// (Req 3.5) - we only drop our handle to it here.
    fn cancel_current(&mut self, out: &mut Vec<LagOrder>) {
        if let Some(rq) = self.resting.take() {
            out.push(LagOrder::cancel(rq.id));
        }
    }

    /// Public pull: cancel any resting entry quote (for Task 7's stale/exit
    /// handling). Does NOT arm the danger watchdog.
    pub fn pull_quote(&mut self, out: &mut Vec<LagOrder>) {
        self.cancel_current(out);
    }

    /// Place a fresh entry quote at `price` on `side`, recording it as resting.
    fn place_new(&mut self, side: Side, price: Price, out: &mut Vec<LagOrder>) {
        let id = self.next_id;
        self.next_id += 1;
        out.push(LagOrder::place(id, side, price, self.cfg.quote_qty));
        self.resting = Some(RestingQuote { id, side, price_raw: price.raw() });
    }

    /// Entry-quote lifecycle while FLAT (place / keep / reprice / pull / cancel).
    /// Assumes the caller has already `observe`d this event. Self-guards on a
    /// not-ready / stale / no-fair-value oracle by pulling any resting quote.
    pub fn manage_entry(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        let (fv, dev) = match (self.oracle.fair_value(), self.oracle.dev_bps()) {
            (Some(fv), Some(dev)) if self.oracle.ready() => (fv, dev),
            _ => {
                self.cancel_current(out);
                return;
            }
        };
        let pos = ctx.position_qty;
        let mag = dev.abs();
        let side_now = Self::reversion_side(dev);

        if let Some(rq) = self.resting {
            let flipped = matches!(side_now, Some(s) if s != rq.side);
            let reverted = mag <= self.cfg.exit_bps;
            let suppressed = !self.inv.allows(pos, rq.side);
            let danger = mag >= self.cfg.danger_bps && side_now == Some(rq.side);

            // Pull on a widening run-over takes priority and arms the watchdog
            // (Req 3.3 / 3.7).
            if danger {
                out.push(LagOrder::cancel(rq.id));
                self.resting = None;
                let deadline = ctx
                    .now
                    .saturating_add(self.cfg.cancel_latency_ns)
                    .saturating_add(self.cfg.ack_timeout_ns);
                self.danger_pull = Some(DangerPull { deadline });
                return;
            }
            // Reverted / flipped / inventory-suppressed -> plain cancel (Req 1.4).
            if reverted || flipped || suppressed {
                out.push(LagOrder::cancel(rq.id));
                self.resting = None;
                return;
            }
            // Still wanted on the same side: keep within tolerance, else reprice.
            let side = rq.side;
            let Some(target) = self.target_price(fv, side, pos) else { return };
            let resting_f = Price::from_raw(rq.price_raw).to_f64();
            let diff_bps = (target.to_f64() - resting_f).abs() / fv * 10_000.0;
            if diff_bps <= self.cfg.reprice_tol_bps {
                return; // keep live, no duplicate (Req 1.3 / 3.1)
            }
            // Reprice = cancel-then-place within one cycle (Req 3.2).
            out.push(LagOrder::cancel(rq.id));
            self.resting = None;
            self.place_new(side, target, out);
            return;
        }

        // No resting quote. Do not re-quote while a danger pull is unresolved.
        if self.danger_pull.is_some() {
            return;
        }
        // Enter only in the band [entry_threshold, danger): below it there is no
        // edge, at/above danger it is a run-over we must not rest into.
        let Some(side) = side_now else { return };
        let want = mag >= self.cfg.entry_threshold_bps
            && mag < self.cfg.danger_bps
            && self.inv.allows(pos, side);
        if want {
            if let Some(target) = self.target_price(fv, side, pos) {
                self.place_new(side, target, out);
            }
        }
    }

    /// If a danger pull is in flight and its `cancel_latency + ack_timeout`
    /// window has elapsed: clear the watchdog, and if the position is non-zero
    /// (the quote filled before the cancel landed - we got run over) return the
    /// Market flatten (Req 3.7). Returns `None` while the window is still open
    /// or once it resolves with no resulting position.
    pub fn take_emergency_flatten(&mut self, ctx: &LagCtx) -> Option<LagOrder> {
        let dp = self.danger_pull?;
        if ctx.now < dp.deadline {
            return None;
        }
        self.danger_pull = None;
        let pos = ctx.position_qty;
        if pos != 0 {
            self.resting = None;
            let side = if pos > 0 { Side::Ask } else { Side::Bid };
            Some(LagOrder::market(side, Qty::from_raw(pos.unsigned_abs() as i64)))
        } else {
            None
        }
    }

    /// Standalone driver used by the unit tests and a simple integration. In
    /// priority order: an emergency flatten if one is due; otherwise on a stale
    /// or missing fair value it pulls any resting quote and places nothing (Req
    /// 2.5); when FLAT it runs the entry lifecycle; while IN POSITION it cancels
    /// any stray entry quote (Task 7 owns the exit). Caller must `observe` first.
    pub fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        if let Some(o) = self.take_emergency_flatten(ctx) {
            out.push(o);
            return;
        }
        if self.oracle.is_stale(ctx.now) || self.oracle.fair_value().is_none() || !self.oracle.ready() {
            self.cancel_current(out);
            return;
        }
        if ctx.position_qty == 0 {
            self.manage_entry(ctx, out);
        } else {
            self.cancel_current(out);
        }
    }
}
#[cfg(test)]
mod quote_tests {
    use super::*;
    use crate::engine::LagOrderKind;
    use forge_book::OrderBook;

    // ---- test-only oracle state forcing ------------------------------------
    // The QuoteManager logic is a pure function of (fair_value, dev, staleness)
    // + inventory + resting state. Warming the rolling baseline through real
    // OrderBook events would make these behavior tests slow and indirect, so we
    // force a known oracle state directly. Leaving the gap ring EMPTY makes the
    // baseline 0, hence fair_value == ref_px exactly - a clean, deterministic
    // anchor for the offset/tolerance math. (Child module => may touch private
    // fields/fns of the parent `maker` module.)
    impl FairValueOracle {
        fn force_for_test(&mut self, now: u64, ref_px: f64, dev: f64) {
            self.ref_px = ref_px;
            self.have_ref = true;
            self.last_ref_ts = now;
            self.cur_dev = dev;
            self.have_dev = true;
        }
    }

    fn oracle() -> FairValueOracle {
        FairValueOracle::new(FairValueConfig::default())
    }

    fn inv(pos_cap: i64, inv_skew_bps: f64, quote_qty: i64) -> InventoryController {
        InventoryController::new(InventoryConfig { pos_cap, inv_skew_bps, quote_qty })
    }

    fn cfg() -> QuoteConfig {
        QuoteConfig {
            quote_offset_bps: 5.0,
            entry_threshold_bps: 16.0,
            exit_bps: 2.0,
            reprice_tol_bps: 1.0,
            danger_bps: 40.0,
            cancel_latency_ns: 100,
            ack_timeout_ns: 100,
            quote_qty: Qty::from_raw(10),
        }
    }

    // An empty book + ref_px 0.0 means observe() would be a no-op, but these
    // tests never call observe (they force the oracle), so the book is just a
    // placeholder to satisfy the borrow.
    fn ctx(now: u64, pos: i64, book: &OrderBook) -> LagCtx<'_> {
        LagCtx { now, exec_book: book, ref_px: 0.0, funding: 0.0, lead_px: 0.0, position_qty: pos }
    }

    fn qm(c: QuoteConfig, pos_cap: i64, skew: f64, q: i64) -> QuoteManager {
        QuoteManager::new(c, oracle(), inv(pos_cap, skew, q))
    }

    #[test]
    fn side_selection_from_dev_sign() {
        // dev > 0 (HL rich) => rest an ASK above fv; dev < 0 => rest a BID below.
        let book = OrderBook::with_max_levels(8);
        let mut up = qm(cfg(), 1000, 0.0, 10);
        up.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        up.on_event(&ctx(100, 0, &book), &mut out);
        assert_eq!(out.len(), 1, "one quote placed");
        assert_eq!(out[0].kind, LagOrderKind::Place);
        assert_eq!(out[0].side, Side::Ask, "dev>0 (HL rich) => reversion side Ask");

        let mut dn = qm(cfg(), 1000, 0.0, 10);
        dn.oracle.force_for_test(100, 1000.0, -20.0);
        let mut out2 = Vec::new();
        dn.on_event(&ctx(100, 0, &book), &mut out2);
        assert_eq!(out2[0].side, Side::Bid, "dev<0 (HL cheap) => reversion side Bid");
    }

    #[test]
    fn offset_places_ask_above_and_bid_below_fv_by_the_right_bps() {
        let book = OrderBook::with_max_levels(8);
        // ASK: fv 1000, offset 5bps => 1000*(1+5/1e4) = 1000.5.
        let mut a = qm(cfg(), 1000, 0.0, 10);
        a.oracle.force_for_test(100, 1000.0, 25.0);
        let mut out = Vec::new();
        a.on_event(&ctx(100, 0, &book), &mut out);
        let px = out[0].price.to_f64();
        assert!((px - 1000.5).abs() < 1e-4, "ask should rest 5bps above fv at 1000.5, got {px}");
        assert!(px > 1000.0, "ask rests ABOVE fair value");

        // BID: fv 1000, offset 5bps => 1000*(1-5/1e4) = 999.5.
        let mut b = qm(cfg(), 1000, 0.0, 10);
        b.oracle.force_for_test(100, 1000.0, -25.0);
        let mut out2 = Vec::new();
        b.on_event(&ctx(100, 0, &book), &mut out2);
        let pxb = out2[0].price.to_f64();
        assert!((pxb - 999.5).abs() < 1e-4, "bid should rest 5bps below fv at 999.5, got {pxb}");
        assert!(pxb < 1000.0, "bid rests BELOW fair value");
    }

    #[test]
    fn keep_within_tol_emits_no_duplicate() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        assert_eq!(out.len(), 1, "initial place");
        assert!(m.has_resting());
        // Same fair value next event: target unchanged => keep, emit nothing.
        let mut out2 = Vec::new();
        m.on_event(&ctx(200, 0, &book), &mut out2);
        assert!(out2.is_empty(), "no duplicate quote within reprice tolerance (Req 1.3/3.1)");
        assert!(m.has_resting());
    }

    #[test]
    fn reprice_when_fair_value_drifts_beyond_tol() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        let old_id = out[0].id;
        // fv jumps to 1003 (dev still rich) => new target ~1003.5, ~30bps from
        // the resting 1000.5 >> 1bp tol => reprice (cancel old + place new).
        m.oracle.force_for_test(200, 1003.0, 20.0);
        let mut out2 = Vec::new();
        m.on_event(&ctx(200, 0, &book), &mut out2);
        assert_eq!(out2.len(), 2, "reprice = cancel + place (Req 3.2)");
        assert_eq!(out2[0].kind, LagOrderKind::Cancel);
        assert_eq!(out2[0].id, old_id, "cancels the OLD resting id");
        assert_eq!(out2[1].kind, LagOrderKind::Place);
        assert!(out2[1].id != old_id, "places a fresh id");
        let np = out2[1].price.to_f64();
        assert!((np - 1003.5015).abs() < 1e-2, "new quote tracks the moved fv, got {np}");
    }

    #[test]
    fn cancel_when_dev_reverts_into_exit_band() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        let id = out[0].id;
        // dev falls to within exit band (|1| <= exit 2) => cancel, no new quote.
        m.oracle.force_for_test(200, 1000.0, 1.0);
        let mut out2 = Vec::new();
        m.on_event(&ctx(200, 0, &book), &mut out2);
        assert_eq!(out2.len(), 1, "just a cancel (Req 1.4)");
        assert_eq!(out2[0].kind, LagOrderKind::Cancel);
        assert_eq!(out2[0].id, id);
        assert!(!m.has_resting());
    }

    #[test]
    fn cancel_when_dev_flips_to_opposite_side() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0); // Ask resting
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        let id = out[0].id;
        // dev flips negative (reversion side now Bid) => cancel the stale Ask.
        m.oracle.force_for_test(200, 1000.0, -20.0);
        let mut out2 = Vec::new();
        m.on_event(&ctx(200, 0, &book), &mut out2);
        assert_eq!(out2.len(), 1, "cancel on side flip (Req 1.4)");
        assert_eq!(out2[0].kind, LagOrderKind::Cancel);
        assert_eq!(out2[0].id, id);
        assert!(!m.has_resting());
    }

    #[test]
    fn pull_on_danger_widen_then_emergency_flatten_if_run_over() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        let id = out[0].id;
        // Basis WIDENS past danger (45 >= 40) on the same side => pull (Req 3.3).
        m.oracle.force_for_test(200, 1000.0, 45.0);
        let mut out2 = Vec::new();
        m.on_event(&ctx(200, 0, &book), &mut out2);
        assert_eq!(out2.len(), 1, "danger pull = a single cancel");
        assert_eq!(out2[0].kind, LagOrderKind::Cancel);
        assert_eq!(out2[0].id, id);
        assert!(!m.has_resting());
        // Before the ack window elapses, no flatten even if we got filled.
        let mut out3 = Vec::new();
        m.on_event(&ctx(250, 10, &book), &mut out3); // deadline = 200+100+100 = 400
        assert!(out3.is_empty(), "no flatten before the ack window elapses");
        // After the window with a non-zero position (cancel lost the race to a
        // fill) => emergency flatten as a Market (Req 3.7).
        let mut out4 = Vec::new();
        m.on_event(&ctx(401, 10, &book), &mut out4);
        assert_eq!(out4.len(), 1, "emergency flatten emitted");
        assert_eq!(out4[0].kind, LagOrderKind::Market);
        assert_eq!(out4[0].side, Side::Ask, "long +10 => flatten by selling (Ask)");
        assert_eq!(out4[0].qty.raw(), 10);
    }

    #[test]
    fn danger_pull_that_lands_clean_does_not_flatten() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        m.oracle.force_for_test(200, 1000.0, 45.0);
        let mut out2 = Vec::new();
        m.on_event(&ctx(200, 0, &book), &mut out2); // pull, deadline 400
        // Window elapses with position still flat (cancel landed): no flatten,
        // watchdog disarmed.
        let mut out3 = Vec::new();
        m.oracle.force_for_test(401, 1000.0, 1.0);
        m.on_event(&ctx(401, 0, &book), &mut out3);
        assert!(out3.is_empty(), "clean pull => no emergency flatten");
        assert!(m.take_emergency_flatten(&ctx(402, 0, &book)).is_none(), "watchdog disarmed");
    }

    #[test]
    fn inventory_cap_suppresses_the_quote() {
        let book = OrderBook::with_max_levels(8);
        // cap 5 < one quote (10) => suppress all quotes (Req 4.7) even on a
        // strong dislocation.
        let mut m = qm(cfg(), 5, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 25.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        assert!(out.is_empty(), "cap smaller than a quote => no quote placed (Req 4.2/4.7)");
        assert!(!m.has_resting());
    }

    #[test]
    fn inventory_skew_shifts_the_target_price() {
        // Long inventory: the reducing side (Ask) rests CLOSER to fv than the
        // base offset, the increasing side (Bid) FURTHER (Req 4.3).
        let m = qm(cfg(), 100, 2.0, 10); // inv_skew 2bps at full cap
        let fv = 1000.0;
        let base_ask = qm(cfg(), 100, 2.0, 10).target_price(fv, Side::Ask, 0).unwrap().to_f64();
        let base_bid = qm(cfg(), 100, 2.0, 10).target_price(fv, Side::Bid, 0).unwrap().to_f64();
        let ask = m.target_price(fv, Side::Ask, 50).unwrap().to_f64(); // skew -1bp
        let bid = m.target_price(fv, Side::Bid, 50).unwrap().to_f64(); // skew +1bp
        assert!((base_ask - 1000.5).abs() < 1e-4 && (base_bid - 999.5).abs() < 1e-4, "flat = symmetric base");
        assert!(ask < base_ask, "long: reducing Ask moves closer to fv ({ask} < {base_ask})");
        assert!(bid < base_bid, "long: increasing Bid moves further below fv ({bid} < {base_bid})");
        // numeric: eff offset ask = 5-1=4bp => 1000.4; bid = 5+1=6bp => 999.4.
        assert!((ask - 1000.4).abs() < 1e-4, "ask eff offset 4bps, got {ask}");
        assert!((bid - 999.4).abs() < 1e-4, "bid eff offset 6bps, got {bid}");
    }

    #[test]
    fn stale_oracle_pulls_and_places_nothing() {
        let book = OrderBook::with_max_levels(8);
        let mut m = qm(cfg(), 1000, 0.0, 10);
        m.oracle.force_for_test(100, 1000.0, 20.0);
        let mut out = Vec::new();
        m.on_event(&ctx(100, 0, &book), &mut out);
        assert!(m.has_resting());
        // Jump far past the staleness limit with no fresh ref => pull, no place.
        let t = 100 + DEFAULT_STALENESS_NS + 1;
        let mut out2 = Vec::new();
        m.on_event(&ctx(t, 0, &book), &mut out2);
        assert_eq!(out2.len(), 1, "stale => cancel resting (Req 2.5)");
        assert_eq!(out2[0].kind, LagOrderKind::Cancel);
        assert!(!m.has_resting());
    }
}
// ===========================================================================
// MakerQuoter (Task 7, Req 1.5/3.7/5.1/5.2) - the exchange-truth state machine.
// ---------------------------------------------------------------------------
// Ties the QuoteManager (entry-quote WHERE/WHEN + danger watchdog) to the
// position/exit half and implements the engine `LagStrategy` trait. It acts on
// EXCHANGE-TRUTH position (`ctx.position_qty`), mirroring the live bot that was
// burned by tracking an internal belief that desynced from the exchange.
//
// STATE MACHINE each event (observe-once, emergency-flatten priority):
//   1. qm.observe(ctx)                                  // sample the oracle once
//   2. if a danger pull lost the race to a fill -> emit the emergency flatten
//      and return (Req 3.7, top priority).
//   3. FLAT (pos == 0): stale/no-fv -> pull any resting quote, place nothing
//      (Req 2.5); else run the QuoteManager entry lifecycle (place/keep/
//      reprice/pull the single ENTRY quote as a maker `Place`).
//   4. IN POSITION (pos != 0): never quote. Cancel any stray resting entry
//      quote; on revert-to-mean (|dev| <= exit_bps) emit a TAKER `Market` exit
//      (Req 5.2); guard against double-submitting that exit while it is in
//      flight. The maker fee on the entry `Place` fill and the taker fee on the
//      `Market` exit are applied by the engine's FeeSchedule (Req 5.1, 5.2) -
//      the strategy just has to use Place for entries and Market for exits.
// ===========================================================================

use crate::engine::LagStrategy;

/// Configuration for the [`MakerQuoter`]. Wraps the [`QuoteConfig`] the inner
/// [`QuoteManager`] needs and adds the strategy-level exit knob. The
/// revert-to-mean exit threshold is `quote.exit_bps` (shared with the entry
/// cancel band, Req 1.4 / 5.2); `hold_ns` is an optional safety backstop.
#[derive(Clone, Copy, Debug, Default)]
pub struct MakerQuoterConfig {
    /// Inner quote-manager configuration (offsets, thresholds, danger, latencies, qty).
    pub quote: QuoteConfig,
    /// Optional hold-timeout TAKER exit (ns). 0 = DISABLED (the default and the
    /// validated behaviour: a pure revert-to-mean exit, Req 5.2). A non-zero
    /// value is a SAFETY backstop only: if a position is opened and the
    /// deviation neither reverts to within `exit_bps` nor widens into the
    /// danger/emergency path (e.g. it lingers in the `[exit, danger)` band, or
    /// the oracle goes stale and never recovers), the position would otherwise
    /// be held indefinitely. Default off so the backtest matches the validated
    /// logic exactly; turn it on for live safety.
    pub hold_ns: u64,
}


/// The maker strategy: drives a [`QuoteManager`] to a full FLAT / IN-POSITION
/// state machine off the exchange-truth position. Entries are resting maker
/// `Place`s; exits are taker `Market`s (so the fee wiring is correct).
pub struct MakerQuoter {
    qm: QuoteManager,
    cfg: MakerQuoterConfig,
    /// Exit-in-flight guard: the signed position for which a taker exit (revert
    /// or emergency) was already submitted but is not yet confirmed by exchange
    /// truth. Keyed on the position so a PARTIAL exit (position shrinks but
    /// stays non-zero) re-evaluates and tops up rather than dead-locking, and a
    /// fresh fill in the same direction is not mistaken for an in-flight exit.
    exiting: Option<i64>,
    /// In-position bookkeeping for the optional hold timeout.
    in_pos: bool,
    entry_ts: u64,
}

impl MakerQuoter {
    /// Build a maker strategy from its config, a fair-value oracle, and an
    /// inventory controller (the latter two are handed straight to the inner
    /// [`QuoteManager`]).
    #[must_use]
    pub fn new(cfg: MakerQuoterConfig, oracle: FairValueOracle, inv: InventoryController) -> Self {
        Self {
            qm: QuoteManager::new(cfg.quote, oracle, inv),
            cfg,
            exiting: None,
            in_pos: false,
            entry_ts: 0,
        }
    }

    /// Fair value passthrough (metrics / later tasks).
    #[must_use]
    pub fn fair_value(&self) -> Option<f64> {
        self.qm.fair_value()
    }

    /// Deviation passthrough (metrics / later tasks).
    #[must_use]
    pub fn dev_bps(&self) -> Option<f64> {
        self.qm.dev_bps()
    }

    /// Side that flattens a signed position: long (+) sells (Ask), short (-) buys (Bid).
    fn exit_side(pos: i64) -> Side {
        if pos > 0 {
            Side::Ask
        } else {
            Side::Bid
        }
    }
}

impl LagStrategy for MakerQuoter {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        // 1. Sample the oracle exactly once, BEFORE any other QuoteManager call.
        self.qm.observe(ctx);

        // 2. Emergency flatten has top priority (Req 3.7): a danger pull that
        //    lost the race to a fill left us exposed; flatten and stop.
        if let Some(flatten) = self.qm.take_emergency_flatten(ctx) {
            out.push(flatten);
            self.exiting = Some(ctx.position_qty);
            return;
        }

        let pos = ctx.position_qty;

        if pos != 0 {
            // IN POSITION (a maker entry quote filled). We NEVER quote here.
            if !self.in_pos {
                self.in_pos = true;
                self.entry_ts = ctx.now;
            }
            // Cancel any stray resting entry quote we still track (it filled, or
            // is no longer wanted now that we hold inventory). The engine no-ops
            // the cancel if the order already filled/gone.
            if self.qm.has_resting() {
                self.qm.pull_quote(out);
            }
            // Do not double-submit the exit while it is in flight for this position.
            if self.exiting == Some(pos) {
                return;
            }
            // Revert-to-mean TAKER exit (Req 5.2): close when the dislocation has
            // reverted to within exit_bps and the oracle is fresh + ready.
            let fresh = self.qm.fair_value().is_some() && !self.qm.is_stale(ctx.now);
            let reverted = fresh
                && matches!(self.qm.dev_bps(), Some(d) if d.abs() <= self.cfg.quote.exit_bps);
            let hold_timeout =
                self.cfg.hold_ns > 0 && ctx.now.saturating_sub(self.entry_ts) >= self.cfg.hold_ns;
            if reverted || hold_timeout {
                out.push(LagOrder::market(
                    Self::exit_side(pos),
                    Qty::from_raw(pos.unsigned_abs() as i64),
                ));
                self.exiting = Some(pos);
            }
        } else {
            // FLAT. Reset the in-position bookkeeping and the exit guard.
            self.in_pos = false;
            self.exiting = None;
            // Stale / no fair value: pull any resting quote, place nothing (Req 2.5).
            // (manage_entry self-guards on no-fv but not on staleness, so gate here.)
            if self.qm.is_stale(ctx.now) || self.qm.fair_value().is_none() {
                self.qm.pull_quote(out);
            } else {
                // Run the entry-quote lifecycle (place / keep / reprice / pull).
                self.qm.manage_entry(ctx, out);
            }
        }
    }
}
#[cfg(test)]
mod maker_quoter_tests {
    use super::*;
    use crate::engine::{LagConfig, LagEngine, LagOrderKind};
    use crate::feed::{LagEvent, LagKind, Role};
    use forge_book::OrderBook;
    use forge_sim::FeeSchedule;

    // Force a known oracle state directly (child module of `maker` => may touch
    // the private oracle fields). Empty gap ring => baseline 0 => fair_value ==
    // ref_px, a clean deterministic anchor; `last_ref_ts = now` keeps it fresh.
    fn force(o: &mut FairValueOracle, now: u64, ref_px: f64, dev: f64) {
        o.ref_px = ref_px;
        o.have_ref = true;
        o.last_ref_ts = now;
        o.cur_dev = dev;
        o.have_dev = true;
    }

    fn quote_cfg() -> QuoteConfig {
        QuoteConfig {
            quote_offset_bps: 5.0,
            entry_threshold_bps: 16.0,
            exit_bps: 2.0,
            reprice_tol_bps: 1.0,
            danger_bps: 40.0,
            cancel_latency_ns: 100,
            ack_timeout_ns: 100,
            quote_qty: Qty::from_raw(10),
        }
    }

    fn make_mq(hold_ns: u64) -> MakerQuoter {
        MakerQuoter::new(
            MakerQuoterConfig { quote: quote_cfg(), hold_ns },
            FairValueOracle::new(FairValueConfig::default()),
            InventoryController::new(InventoryConfig { pos_cap: 1_000, inv_skew_bps: 0.0, quote_qty: 10 }),
        )
    }

    // ref_px 0.0 so the on_event observe() is a no-op for the oracle (these unit
    // tests force the oracle directly); the book is just a borrow placeholder.
    fn ctx(now: u64, pos: i64, book: &OrderBook) -> LagCtx<'_> {
        LagCtx { now, exec_book: book, ref_px: 0.0, funding: 0.0, lead_px: 0.0, position_qty: pos }
    }

    #[test]
    fn flat_dislocation_places_a_maker_entry_on_the_reversion_side() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0); // HL rich => Ask
        let mut out = Vec::new();
        mq.on_event(&ctx(100, 0, &book), &mut out);
        assert_eq!(out.len(), 1, "FLAT dislocation places exactly one entry quote");
        assert_eq!(out[0].kind, LagOrderKind::Place, "entries are resting maker Places (Req 5.1)");
        assert_eq!(out[0].side, Side::Ask, "dev>0 (HL rich) => rest an Ask");
        assert!(mq.qm.has_resting());
    }

    #[test]
    fn in_position_exits_with_a_taker_market_on_revert_to_mean() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        // Short -10, deviation reverted to within exit band => taker flatten.
        force(&mut mq.qm.oracle, 100, 1000.0, 1.0);
        let mut out = Vec::new();
        mq.on_event(&ctx(100, -10, &book), &mut out);
        assert_eq!(out.len(), 1, "one taker exit");
        assert_eq!(out[0].kind, LagOrderKind::Market, "exit is a TAKER market (Req 5.2)");
        assert_eq!(out[0].side, Side::Bid, "short => flatten by buying (Bid)");
        assert_eq!(out[0].qty.raw(), 10, "flatten the whole position");
    }

    #[test]
    fn in_position_holds_and_never_quotes_when_not_reverted() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        // Still dislocated (|dev| 20 > exit 2): hold, emit nothing, NEVER quote.
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0);
        let mut out = Vec::new();
        mq.on_event(&ctx(100, -10, &book), &mut out);
        assert!(out.is_empty(), "in position + not reverted => hold, no exit, no quote");
        assert!(!out.iter().any(|o| o.kind == LagOrderKind::Place), "never places an entry while in position");
    }

    #[test]
    fn in_position_cancels_a_stray_resting_entry_quote() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        // FLAT: place an entry quote.
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0);
        let mut out1 = Vec::new();
        mq.on_event(&ctx(100, 0, &book), &mut out1);
        assert_eq!(out1[0].kind, LagOrderKind::Place);
        assert!(mq.qm.has_resting());
        // Now we are filled (pos != 0) but still track the resting quote => cancel
        // the stray, and (still dislocated) do NOT exit yet.
        force(&mut mq.qm.oracle, 200, 1000.0, 20.0);
        let mut out2 = Vec::new();
        mq.on_event(&ctx(200, -10, &book), &mut out2);
        assert_eq!(out2.len(), 1, "just the stray cancel");
        assert_eq!(out2[0].kind, LagOrderKind::Cancel);
        assert!(!mq.qm.has_resting());
    }

    #[test]
    fn does_not_double_submit_the_exit_while_in_flight() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        force(&mut mq.qm.oracle, 100, 1000.0, 1.0);
        let mut out1 = Vec::new();
        mq.on_event(&ctx(100, -10, &book), &mut out1);
        assert_eq!(out1.len(), 1, "first event submits the exit");
        assert_eq!(out1[0].kind, LagOrderKind::Market);
        // Same position still showing (exit in flight): no second market.
        force(&mut mq.qm.oracle, 200, 1000.0, 1.0);
        let mut out2 = Vec::new();
        mq.on_event(&ctx(200, -10, &book), &mut out2);
        assert!(out2.is_empty(), "no double exit while the flatten is in flight");
    }

    #[test]
    fn emergency_flatten_takes_priority_when_a_danger_pull_loses_the_race() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        // FLAT: place an Ask.
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0);
        let mut o1 = Vec::new();
        mq.on_event(&ctx(100, 0, &book), &mut o1);
        assert_eq!(o1[0].kind, LagOrderKind::Place);
        // Basis WIDENS past danger (45 >= 40) => danger pull arms the watchdog
        // (deadline = 200 + cancel_latency 100 + ack_timeout 100 = 400).
        force(&mut mq.qm.oracle, 200, 1000.0, 45.0);
        let mut o2 = Vec::new();
        mq.on_event(&ctx(200, 0, &book), &mut o2);
        assert_eq!(o2.len(), 1, "danger pull = single cancel");
        assert_eq!(o2[0].kind, LagOrderKind::Cancel);
        // After the window with a non-zero position (the cancel lost the race to
        // a fill): emergency flatten fires with top priority as a Market.
        force(&mut mq.qm.oracle, 401, 1000.0, 45.0);
        let mut o3 = Vec::new();
        mq.on_event(&ctx(401, -10, &book), &mut o3);
        assert_eq!(o3.len(), 1, "emergency flatten emitted");
        assert_eq!(o3[0].kind, LagOrderKind::Market, "emergency flatten is a Market (Req 3.7)");
        assert_eq!(o3[0].side, Side::Bid, "short -10 => flatten by buying (Bid)");
        assert_eq!(o3[0].qty.raw(), 10);
    }

    #[test]
    fn optional_hold_timeout_forces_a_taker_exit() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(50); // 50ns hold backstop
        // Still dislocated (not reverted) but the hold elapses => safety exit.
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0);
        let mut o1 = Vec::new();
        mq.on_event(&ctx(100, 10, &book), &mut o1); // entry_ts = 100, hold not elapsed
        assert!(o1.is_empty(), "no exit before the hold elapses");
        force(&mut mq.qm.oracle, 160, 1000.0, 20.0);
        let mut o2 = Vec::new();
        mq.on_event(&ctx(160, 10, &book), &mut o2); // elapsed 60 >= 50
        assert_eq!(o2.len(), 1, "hold timeout forces an exit");
        assert_eq!(o2[0].kind, LagOrderKind::Market);
        assert_eq!(o2[0].side, Side::Ask, "long +10 => flatten by selling (Ask)");
    }

    #[test]
    fn flat_stale_oracle_pulls_resting_and_places_nothing() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq(0);
        // Place an entry at t=100 (force sets last_ref_ts = 100).
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0);
        let mut o1 = Vec::new();
        mq.on_event(&ctx(100, 0, &book), &mut o1);
        assert!(mq.qm.has_resting());
        // Jump far past the staleness limit WITHOUT forcing again (so last_ref_ts
        // stays 100); the on_event observe() is a no-op (ctx.ref_px == 0.0).
        let t = 100 + DEFAULT_STALENESS_NS + 1;
        let mut o2 = Vec::new();
        mq.on_event(&ctx(t, 0, &book), &mut o2);
        assert_eq!(o2.len(), 1, "stale => cancel resting (Req 2.5)");
        assert_eq!(o2[0].kind, LagOrderKind::Cancel);
        assert!(!mq.qm.has_resting(), "no quote placed while stale");
    }

    // ---- END-TO-END through the real LagEngine -----------------------------

    fn lev(role: Role, kind: LagKind, side: Side, px: f64, qty: f64, ts: u64) -> LagEvent {
        LagEvent {
            role,
            kind,
            exch_ts: ts,
            local_ts: ts,
            side: Some(side),
            price: Price::from_f64(px).unwrap(),
            qty: Qty::from_f64(qty).unwrap(),
            src: 0,
            aux: 0.0,
        }
    }

    // A scripted stream that drives ONE full maker round trip through the real
    // engine. The HL book is FIXED (bid 999.9 / ask 1000.1, micro ~1000); the
    // dislocation is created by dropping the OKX REFERENCE price, so there are no
    // messy multi-level book transitions to perturb the microprice.
    //   warm-up : ref 1000 (baseline ~0, oracle warms)
    //   disloc  : ref 998  => HL rich ~20bps => dev>0 => rest an Ask above fv
    //   fill    : an HL buy prints through the resting ask => maker fill (short)
    //   revert  : ref 1000 => |dev| ~0 <= exit => taker market exit => flat
    fn round_trip_stream() -> Vec<LagEvent> {
        let size = 100.0;
        let mut evs = Vec::new();
        let mut ts = 1_000_000_000u64;
        let step = 1_000_000u64; // 1 ms
        // Fixed HL book.
        evs.push(lev(Role::Exec, LagKind::BookDelta, Side::Bid, 999.9, size, ts));
        evs.push(lev(Role::Exec, LagKind::BookDelta, Side::Ask, 1000.1, size, ts));
        // Warm-up: ref at 1000 for plenty of samples (> MIN_DEV_SAMPLES).
        for _ in 0..45 {
            ts += step;
            evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
        }
        // Dislocation: ref drops to 998 => HL rich ~20bps => rest an Ask.
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 998.0, size, ts));
        // Through-trade: an HL BUY printing above the resting ask => maker fill.
        ts += step;
        evs.push(lev(Role::Exec, LagKind::Trade, Side::Bid, 1001.0, 50.0, ts));
        // Hold one beat, still dislocated.
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 998.0, size, ts));
        // Reversion: ref back to 1000 => dev ~0 => taker exit.
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
        // Trailing beats so the exit market drains and fills, returning to flat.
        for _ in 0..6 {
            ts += step;
            evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
        }
        evs
    }

    fn e2e_strategy() -> MakerQuoter {
        // Small sample cadence so the oracle warms within the 1ms-spaced stream.
        let oracle = FairValueOracle::new(FairValueConfig {
            top_n: 5,
            window: 500,
            sample_ns: 1_000_000,
            staleness_ns: DEFAULT_STALENESS_NS,
        });
        let qty = Qty::from_f64(0.1).unwrap();
        let cfg = MakerQuoterConfig {
            quote: QuoteConfig {
                quote_offset_bps: 25.0, // rest the ask well above the rich book (non-marketable)
                entry_threshold_bps: 16.0,
                exit_bps: 2.0,
                reprice_tol_bps: 1.0,
                danger_bps: 40.0,
                cancel_latency_ns: 0,
                ack_timeout_ns: 0,
                quote_qty: qty,
            },
            hold_ns: 0,
        };
        let inv = InventoryController::new(InventoryConfig {
            pos_cap: Qty::from_f64(10.0).unwrap().raw(),
            inv_skew_bps: 0.0,
            quote_qty: qty.raw(),
        });
        MakerQuoter::new(cfg, oracle, inv)
    }

    fn run_round_trip() -> crate::engine::LagReport {
        let evs = round_trip_stream();
        let mut eng = LagEngine::new(
            e2e_strategy(),
            LagConfig { order_latency_ns: 0, cancel_latency_ns: 0, exec_book_levels: 20, fees: FeeSchedule::legacy() },
        );
        eng.run(evs.iter()).unwrap();
        eng.finish()
    }

    #[test]
    fn end_to_end_one_full_maker_round_trip_through_the_engine() {
        let r = run_round_trip();
        eprintln!(
            "[maker e2e] events={} submitted={} filled={} maker_fills={} round_trips={} final_pos={} net={:.6}",
            r.events, r.orders_submitted, r.orders_filled, r.maker_fills, r.round_trips, r.final_position, r.net_pnl
        );
        assert!(r.maker_fills >= 1, "the entry must fill as a MAKER (Req 5.1); maker_fills={}", r.maker_fills);
        assert!(
            r.orders_filled >= 2,
            "a maker entry fill + a taker exit fill = 2 fills; got {}",
            r.orders_filled
        );
        assert_eq!(r.round_trips, 1, "exactly one completed round trip recorded");
        assert_eq!(r.final_position, 0, "the taker exit returns us to flat");
    }

    #[test]
    fn end_to_end_round_trip_is_deterministic() {
        let a = run_round_trip();
        let b = run_round_trip();
        assert_eq!(a.round_trips, b.round_trips, "round trips identical across runs (Req 9.5)");
        assert_eq!(a.maker_fills, b.maker_fills, "maker fills identical across runs");
        assert_eq!(a.orders_filled, b.orders_filled, "fills identical across runs");
        assert_eq!(a.final_position, b.final_position, "final position identical across runs");
        assert_eq!(a.net_pnl.to_bits(), b.net_pnl.to_bits(), "net P&L byte-identical across runs (Req 9.5)");
    }
}

#[cfg(test)]
mod maker_quoter_stale_tests {
    use super::*;
    use crate::engine::LagOrderKind;
    use forge_book::OrderBook;

    fn quote_cfg() -> QuoteConfig {
        QuoteConfig {
            quote_offset_bps: 5.0,
            entry_threshold_bps: 16.0,
            exit_bps: 2.0,
            reprice_tol_bps: 1.0,
            danger_bps: 40.0,
            cancel_latency_ns: 100,
            ack_timeout_ns: 100,
            quote_qty: Qty::from_raw(10),
        }
    }

    fn make_mq() -> MakerQuoter {
        MakerQuoter::new(
            MakerQuoterConfig { quote: quote_cfg(), hold_ns: 0 },
            FairValueOracle::new(FairValueConfig::default()),
            InventoryController::new(InventoryConfig { pos_cap: 1_000, inv_skew_bps: 0.0, quote_qty: 10 }),
        )
    }

    fn force(o: &mut FairValueOracle, now: u64, ref_px: f64, dev: f64) {
        o.ref_px = ref_px;
        o.have_ref = true;
        o.last_ref_ts = now;
        o.cur_dev = dev;
        o.have_dev = true;
    }

    fn ctx(now: u64, pos: i64, book: &OrderBook) -> LagCtx<'_> {
        LagCtx { now, exec_book: book, ref_px: 0.0, funding: 0.0, lead_px: 0.0, position_qty: pos }
    }

    // Task-7 scenario: while IN POSITION with a STALE oracle, the strategy must
    // still PULL any stray resting entry quote (Req 2.5) but must NOT take the
    // revert-to-mean exit off an untrustworthy (stale) deviation; once the
    // oracle is FRESH again and the gap has reverted, the taker exit fires.
    #[test]
    fn stale_in_position_pulls_stray_quote_then_exits_on_revert_when_fresh() {
        let book = OrderBook::with_max_levels(8);
        let mut mq = make_mq();
        // FLAT: place an entry quote (force sets last_ref_ts = 100).
        force(&mut mq.qm.oracle, 100, 1000.0, 20.0);
        let mut o1 = Vec::new();
        mq.on_event(&ctx(100, 0, &book), &mut o1);
        assert_eq!(o1[0].kind, LagOrderKind::Place);
        assert!(mq.qm.has_resting());

        // IN POSITION but oracle STALE (now far past last_ref_ts=100, no refresh;
        // the on_event observe() is a no-op as ctx.ref_px == 0.0): pull the stray
        // resting quote, and DO NOT exit on a stale deviation.
        let t_stale = 100 + DEFAULT_STALENESS_NS + 1;
        let mut o2 = Vec::new();
        mq.on_event(&ctx(t_stale, -10, &book), &mut o2);
        assert_eq!(o2.len(), 1, "stale in-position pulls the stray quote only");
        assert_eq!(o2[0].kind, LagOrderKind::Cancel, "stray entry quote cancelled (Req 2.5)");
        assert!(!mq.qm.has_resting());
        assert!(!o2.iter().any(|o| o.kind == LagOrderKind::Market), "no exit while stale");

        // Oracle FRESH again and reverted within the exit band => taker exit now.
        let t_fresh = t_stale + 1;
        force(&mut mq.qm.oracle, t_fresh, 1000.0, 1.0);
        let mut o3 = Vec::new();
        mq.on_event(&ctx(t_fresh, -10, &book), &mut o3);
        assert_eq!(o3.len(), 1, "fresh + reverted => exit");
        assert_eq!(o3[0].kind, LagOrderKind::Market, "revert-to-mean taker exit (Req 5.2)");
        assert_eq!(o3[0].side, Side::Bid, "short -10 => flatten by buying (Bid)");
        assert_eq!(o3[0].qty.raw(), 10);
    }
}

// ===========================================================================
// Task 8: config-range validation (Req 3.6) + fee-economics finding (Req 5.x).
// ===========================================================================
// VALIDATION API - two reachable layers (the hunt CLI in Task 11 calls these):
//
//   * Per-config `validate()` on [`FairValueConfig`] / [`InventoryConfig`] /
//     [`QuoteConfig`] / [`MakerQuoterConfig`] -> `Result<(), String>`. Each
//     REJECTS an out-of-range value and returns an error that NAMES the
//     offending parameter, its value, and the valid range (Req 3.6).
//
//   * [`MakerQuoter::try_new`] - the VALIDATED constructor the CLI should call:
//     validates all three configs AND the cross-config `quote_qty` consistency
//     (the quote size lives in both [`QuoteConfig`] as a `Qty` and in
//     [`InventoryConfig`] as a raw `i64`; the cap check and fill accounting only
//     agree if they are EQUAL), then builds via the infallible `new`s.
//
// THE INFALLIBLE `*::new` CONSTRUCTORS ARE KEPT on purpose. `FairValueOracle::new`,
// `InventoryController::new`, `QuoteManager::new`, and `MakerQuoter::new` FLOOR
// their inputs (e.g. `top_n.max(1)`, `pos_cap.max(0)`) instead of rejecting, and
// existing components/tests rely on that. Use the infallible `new` for INTERNAL
// construction where inputs are already trusted; use `try_new` / `validate` at
// the CONFIG BOUNDARY (CLI / external input) where a bad value must be REPORTED
// (naming the param) rather than silently floored.
//
// VALID RANGES (from Req 1.5/3.6 and the design doc):
//   quote_offset_bps   0..=100        entry_threshold_bps 1..=100
//   exit_bps           0..=100 and < entry_threshold_bps (revert band inside entry band)
//   reprice_tol_bps    0.1..=100.0    danger_bps          1.0..=500.0
//   cancel_latency_ns  0..=5_000_000_000 (0-5000 ms)
//   ack_timeout_ns     0..=5_000_000_000 (0-5000 ms)
//   pos_cap            >= 0           inv_skew_bps        >= 0
//   quote_qty          > 0, and QuoteConfig.quote_qty == InventoryConfig.quote_qty
//   FairValueConfig: top_n/window/sample_ns >= 1, staleness_ns >= 1.

/// Largest accepted latency/timeout knob: 5000 ms expressed in nanoseconds.
pub const MAX_LATENCY_NS: u64 = 5_000_000_000;

/// Reject `v` unless it lies in `[lo, hi]` (NaN is always rejected), naming `name`.
fn check_f64(name: &str, v: f64, lo: f64, hi: f64) -> Result<(), String> {
    if v.is_nan() || v < lo || v > hi {
        return Err(format!("{name} {v} out of range {lo}..={hi}"));
    }
    Ok(())
}

/// Reject `v` unless it lies in `[0, hi]`, naming `name`.
fn check_u64(name: &str, v: u64, hi: u64) -> Result<(), String> {
    if v > hi {
        return Err(format!("{name} {v} out of range 0..={hi}"));
    }
    Ok(())
}

impl FairValueConfig {
    /// Validate the oracle knobs, REJECTING out-of-range values with an error
    /// that names the bad parameter (Req 3.6). `top_n`, `window`, `sample_ns`
    /// must be at least 1; `staleness_ns` must be at least 1 (a 0 limit would
    /// mark every read stale). The infallible [`FairValueOracle::new`] floors
    /// these instead - this is the boundary check the CLI uses.
    ///
    /// # Errors
    /// Returns the name + value + range of the first out-of-range parameter.
    pub fn validate(&self) -> Result<(), String> {
        if self.top_n < 1 {
            return Err(format!("top_n {} out of range >=1", self.top_n));
        }
        if self.window < 1 {
            return Err(format!("window {} out of range >=1", self.window));
        }
        if self.sample_ns < 1 {
            return Err(format!("sample_ns {} out of range >=1", self.sample_ns));
        }
        if self.staleness_ns < 1 {
            return Err(format!("staleness_ns {} out of range >=1", self.staleness_ns));
        }
        Ok(())
    }
}

impl InventoryConfig {
    /// Validate the inventory knobs (Req 4.5), REJECTING out-of-range values
    /// with an error naming the bad parameter (Req 3.6): `pos_cap >= 0`,
    /// `inv_skew_bps >= 0`, `quote_qty > 0`.
    ///
    /// # Errors
    /// Returns the name + value of the first out-of-range parameter.
    pub fn validate(&self) -> Result<(), String> {
        if self.pos_cap < 0 {
            return Err(format!("pos_cap {} out of range >=0", self.pos_cap));
        }
        if self.inv_skew_bps.is_nan() || self.inv_skew_bps < 0.0 {
            return Err(format!("inv_skew_bps {} out of range >=0", self.inv_skew_bps));
        }
        if self.quote_qty <= 0 {
            return Err(format!("quote_qty {} out of range >0", self.quote_qty));
        }
        Ok(())
    }
}

impl QuoteConfig {
    /// Validate the quote-manager knobs, REJECTING out-of-range values with an
    /// error naming the bad parameter (Req 3.6). See the module-level VALID
    /// RANGES table. Also enforces the SENSIBLE cross-field rule that the
    /// revert-to-mean exit band sits strictly inside the entry band
    /// (`exit_bps < entry_threshold_bps`), so a position can actually be entered
    /// before it is eligible to exit.
    ///
    /// # Errors
    /// Returns the name + value + range of the first out-of-range parameter (or
    /// the offending relationship for the exit/entry rule).
    pub fn validate(&self) -> Result<(), String> {
        check_f64("quote_offset_bps", self.quote_offset_bps, 0.0, 100.0)?;
        check_f64("entry_threshold_bps", self.entry_threshold_bps, 1.0, 100.0)?;
        check_f64("exit_bps", self.exit_bps, 0.0, 100.0)?;
        check_f64("reprice_tol_bps", self.reprice_tol_bps, 0.1, 100.0)?;
        check_f64("danger_bps", self.danger_bps, 1.0, 500.0)?;
        check_u64("cancel_latency_ns", self.cancel_latency_ns, MAX_LATENCY_NS)?;
        check_u64("ack_timeout_ns", self.ack_timeout_ns, MAX_LATENCY_NS)?;
        if self.quote_qty.raw() <= 0 {
            return Err(format!("quote_qty {} out of range >0", self.quote_qty.raw()));
        }
        if self.exit_bps >= self.entry_threshold_bps {
            return Err(format!(
                "exit_bps {} must be < entry_threshold_bps {}",
                self.exit_bps, self.entry_threshold_bps
            ));
        }
        Ok(())
    }
}

impl MakerQuoterConfig {
    /// Validate the maker-strategy config. Delegates to [`QuoteConfig::validate`]
    /// for the inner quote knobs (Req 3.6). `hold_ns` is an optional safety
    /// backstop with no upper bound (0 = disabled, the validated default), so it
    /// is intentionally unconstrained here.
    ///
    /// # Errors
    /// Propagates the first error from the inner [`QuoteConfig::validate`].
    pub fn validate(&self) -> Result<(), String> {
        self.quote.validate()
    }
}
impl MakerQuoter {
    /// VALIDATED constructor (Req 3.6 / 1.5) - the path the hunt CLI (Task 11)
    /// should use. Validates the [`MakerQuoterConfig`] (inner [`QuoteConfig`]),
    /// the [`FairValueConfig`], and the [`InventoryConfig`], then checks the
    /// cross-config `quote_qty` consistency before building via the infallible
    /// [`MakerQuoter::new`]. On any out-of-range value it returns an error that
    /// NAMES the offending parameter; it never silently floors.
    ///
    /// # Errors
    /// - any single out-of-range knob (named, with its value and range), or
    /// - a `quote_qty` mismatch between [`QuoteConfig`] (a `Qty`) and
    ///   [`InventoryConfig`] (a raw `i64`) - they must be EQUAL so the cap check
    ///   and the fill accounting agree.
    pub fn try_new(
        cfg: MakerQuoterConfig,
        fv_cfg: FairValueConfig,
        inv_cfg: InventoryConfig,
    ) -> Result<Self, String> {
        cfg.validate()?;
        fv_cfg.validate()?;
        inv_cfg.validate()?;
        if cfg.quote.quote_qty.raw() != inv_cfg.quote_qty {
            return Err(format!(
                "quote_qty mismatch: QuoteConfig.quote_qty {} != InventoryConfig.quote_qty {}",
                cfg.quote.quote_qty.raw(),
                inv_cfg.quote_qty
            ));
        }
        Ok(Self::new(cfg, FairValueOracle::new(fv_cfg), InventoryController::new(inv_cfg)))
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn good_quote() -> QuoteConfig {
        QuoteConfig {
            quote_offset_bps: 2.0,
            entry_threshold_bps: 16.0,
            exit_bps: 2.0,
            reprice_tol_bps: 1.0,
            danger_bps: 40.0,
            cancel_latency_ns: 1_000_000_000,
            ack_timeout_ns: 1_000_000_000,
            quote_qty: Qty::from_raw(10),
        }
    }
    fn good_inv() -> InventoryConfig {
        InventoryConfig { pos_cap: 1_000, inv_skew_bps: 1.0, quote_qty: 10 }
    }
    fn good_fv() -> FairValueConfig {
        FairValueConfig::default()
    }

    fn assert_err_names(res: Result<(), String>, needle: &str) {
        let e = res.expect_err("expected an out-of-range rejection");
        assert!(e.contains(needle), "error should name `{needle}`, got: {e}");
    }

    #[test]
    fn good_config_is_accepted() {
        // Each per-config validate passes, and the validated constructor builds.
        assert!(good_quote().validate().is_ok());
        assert!(good_inv().validate().is_ok());
        assert!(good_fv().validate().is_ok());
        let mq = MakerQuoter::try_new(
            MakerQuoterConfig { quote: good_quote(), hold_ns: 0 },
            good_fv(),
            good_inv(),
        );
        assert!(mq.is_ok(), "a fully valid config must build: {:?}", mq.err());
    }

    #[test]
    fn rejects_quote_offset_above_range() {
        let mut c = good_quote();
        c.quote_offset_bps = 101.0; // > 100
        assert_err_names(c.validate(), "quote_offset_bps");
    }

    #[test]
    fn rejects_entry_threshold_zero() {
        let mut c = good_quote();
        c.entry_threshold_bps = 0.0; // < 1
        assert_err_names(c.validate(), "entry_threshold_bps");
    }

    #[test]
    fn rejects_danger_below_floor() {
        let mut c = good_quote();
        c.danger_bps = 0.5; // < 1.0
        assert_err_names(c.validate(), "danger_bps");
    }

    #[test]
    fn rejects_reprice_tol_below_floor() {
        let mut c = good_quote();
        c.reprice_tol_bps = 0.05; // < 0.1
        assert_err_names(c.validate(), "reprice_tol_bps");
    }

    #[test]
    fn rejects_cancel_latency_too_big() {
        let mut c = good_quote();
        c.cancel_latency_ns = MAX_LATENCY_NS + 1; // > 5000 ms
        assert_err_names(c.validate(), "cancel_latency_ns");
    }

    #[test]
    fn rejects_ack_timeout_too_big() {
        let mut c = good_quote();
        c.ack_timeout_ns = 6_000_000_000; // > 5000 ms
        assert_err_names(c.validate(), "ack_timeout_ns");
    }

    #[test]
    fn rejects_nan_offset() {
        // NaN is out of range for any bounded f64 knob (Req 3.6 fail-fast).
        let mut c = good_quote();
        c.quote_offset_bps = f64::NAN;
        assert_err_names(c.validate(), "quote_offset_bps");
    }

    #[test]
    fn rejects_exit_not_strictly_below_entry() {
        let mut c = good_quote();
        c.exit_bps = 20.0; // >= entry_threshold 16 => the exit band swallows entry
        assert_err_names(c.validate(), "exit_bps");
    }

    #[test]
    fn rejects_negative_pos_cap() {
        let mut c = good_inv();
        c.pos_cap = -1;
        assert_err_names(c.validate(), "pos_cap");
    }

    #[test]
    fn rejects_negative_inv_skew() {
        let mut c = good_inv();
        c.inv_skew_bps = -0.1;
        assert_err_names(c.validate(), "inv_skew_bps");
    }

    #[test]
    fn rejects_zero_quote_qty_in_inventory() {
        let mut c = good_inv();
        c.quote_qty = 0; // must be > 0
        assert_err_names(c.validate(), "quote_qty");
    }

    #[test]
    fn rejects_zero_staleness() {
        let mut c = good_fv();
        c.staleness_ns = 0; // a 0 limit would mark every read stale
        assert_err_names(c.validate(), "staleness_ns");
    }

    #[test]
    fn rejects_mismatched_quote_qty_across_configs() {
        // QuoteConfig.quote_qty (10) must equal InventoryConfig.quote_qty.
        let mut inv = good_inv();
        inv.quote_qty = 7; // mismatch vs QuoteConfig's 10
        let res = MakerQuoter::try_new(
            MakerQuoterConfig { quote: good_quote(), hold_ns: 0 },
            good_fv(),
            inv,
        );
        let e = res.err().expect("a quote_qty mismatch must be rejected");
        assert!(e.contains("quote_qty mismatch"), "error should name the mismatch, got: {e}");
        assert!(e.contains("10") && e.contains("7"), "error should show both values, got: {e}");
    }

    #[test]
    fn try_new_propagates_a_bad_inner_quote_knob() {
        // A bad QuoteConfig knob is surfaced (named) through the constructor.
        let mut q = good_quote();
        q.danger_bps = 600.0; // > 500
        let res = MakerQuoter::try_new(
            MakerQuoterConfig { quote: q, hold_ns: 0 },
            good_fv(),
            good_inv(),
        );
        let e = res.err().expect("bad inner knob must reject");
        assert!(e.contains("danger_bps"), "constructor names the bad inner knob, got: {e}");
    }
}

#[cfg(test)]
mod fee_economics_tests {
    // FEE ECONOMICS FINDING (Req 5.3/5.4/5.5/5.6).
    //
    // Maker entry fills use the maker fee/rebate and taker (market) exits use
    // the taker fee, applied by the engine's `FeeSchedule` via
    // `Account::apply_maker` / `Account::apply_taker` - already wired in
    // `engine.rs` (`process_trade` -> `apply_maker`, `fill_taker` -> `apply_taker`).
    //
    // Req 5.6 asks for a FAIL-FAST when the maker fee/rebate is "not defined" in
    // the active fee schedule at a maker fill. INVESTIGATION of the sacred-core
    // `forge_sim::FeeSchedule` shows it stores the maker rate as a PLAIN `i64`
    // field (`maker_rate_raw`), NOT an `Option`. A maker fee is therefore ALWAYS
    // defined by construction - the "missing" case cannot occur. We satisfy
    // Req 5.6 BY CONSTRUCTION (the type makes the bad state unrepresentable)
    // rather than by adding a contrived runtime check, and we do NOT modify the
    // sacred core. The tests below assert that invariant honestly.
    use forge_core::SCALE;
    use forge_sim::{money_to_f64, FeeSchedule};

    /// A notional of 1000 quote units, scaled by `SCALE` (the `Money` unit).
    fn notional() -> i128 {
        1_000_i128 * i128::from(SCALE)
    }

    #[test]
    fn legacy_schedule_yields_maker_rebate_and_distinct_taker_fee() {
        let fees = FeeSchedule::legacy();
        let n = notional();
        let maker = fees.maker_fee(n);
        let taker = fees.taker_fee(n);
        // Req 5.5: the legacy maker rate is a REBATE -> maker_fee is NEGATIVE,
        // i.e. a credit that INCREASES net proceeds when treated as a cost.
        assert!(maker < 0, "legacy maker fee is a rebate/credit, got {}", money_to_f64(maker));
        // Req 5.2/5.3: the taker fee is a real DEBIT (> 0) that decreases proceeds.
        assert!(taker > 0, "taker fee is a debit, got {}", money_to_f64(taker));
        // Maker and taker economics are DISTINCT (the maker bar is easier).
        assert_ne!(maker, taker, "maker and taker fees must differ");
        // Exact legacy numbers on 1000 notional: maker -0.002% = -0.02; taker 0.025% = 0.25.
        assert!((money_to_f64(maker) + 0.02).abs() < 1e-9, "maker -0.02, got {}", money_to_f64(maker));
        assert!((money_to_f64(taker) - 0.25).abs() < 1e-9, "taker 0.25, got {}", money_to_f64(taker));
    }

    #[test]
    fn maker_fee_is_defined_by_construction_so_req_5_6_is_vacuous() {
        // FeeSchedule has no Option-typed fee; maker_fee/taker_fee return a
        // defined Money for ANY notional and ANY schedule. There is no
        // "missing maker fee" state to fail-fast on (Req 5.6 satisfied by
        // construction). Exercise several schedules to document that.
        for fees in [FeeSchedule::legacy(), FeeSchedule::zero(), FeeSchedule::new(25_000, -2_000)] {
            let _maker = fees.maker_fee(notional()); // always defined, never panics/None
            let _taker = fees.taker_fee(notional());
        }
    }

    #[test]
    fn zero_schedule_has_no_rebate_and_no_fee() {
        // The zero schedule (used to isolate spread in tests) credits/debits 0:
        // confirms maker_fee is still defined (just 0), reinforcing 5.6 vacuity.
        let fees = FeeSchedule::zero();
        assert_eq!(fees.maker_fee(notional()), 0);
        assert_eq!(fees.taker_fee(notional()), 0);
    }
}