//! `sweepscope` - a RESEARCH / ANALYSIS tool (NOT a strategy) that DETECTS and
//! CHARACTERISES the classic "stop-run / liquidity-sweep" setup on a bigger
//! (swing) timescale, and answers the trader's three questions HONESTLY:
//!   1. after a SIDEWAYS range gets SWEPT past an edge, what is the
//!      CONTINUATION-vs-REVERSAL split + the sizes (bps) of each?
//!   2. can an ORDERFLOW CONFIRM (absorption vs follow-through, read <= entry)
//!      separate the two AHEAD of time (with valid knob-bite)?
//!   3. does EITHER trade - the REVERSAL (maker entry at the swept edge) or the
//!      CONTINUATION (taker) - clear NET-positive, fees + run-overs included, at
//!      a TRUSTABLE n (>~30) with a favourable reward:risk?
//!
//! This is the bigger-per-trade swing lead (tens-to-hundreds of bps targets),
//! deliberately LEAVING the tick-scale microstructure that is dead at the ~9bps
//! taker fee. The setup, mechanised WITHOUT presupposing direction:
//!   * SIDEWAYS RANGE: over a lookback L, the HL microprice high/low form a range
//!     [lo,hi]; it must be genuinely sideways - width (hi-lo)/mid*1e4 <= range-max
//!     bps (skip if already trending).
//!   * SWEEP (the manipulation): price pokes BEYOND an edge by a margin S bps
//!     (above hi = UP sweep, below lo = DOWN sweep) - the break past the resting
//!     edge where stops/liquidations sit. De-duped by a cooldown.
//!   * OUTCOME (measured, presupposing NOTHING): from the sweep extreme, over a
//!     forward horizon, classify CONTINUATION (extends further by >= a threshold
//!     before returning) vs REVERSAL (snaps back through the range edge), and
//!     report the % split + magnitudes + times.
//!   * TWO honest first-touch trade sims (NO-lookahead, run-overs INCLUDED, real
//!     fees): (A) REVERSAL - enter AGAINST the sweep (UP->short / DOWN->long);
//!     the entry can be a MAKER fill (rest a limit AT the swept edge, the sweep
//!     trades THROUGH it) = the fee escape (~6bps RT vs ~9bps taker); a taker
//!     variant too. (B) CONTINUATION - enter WITH the break (taker, ~9bps RT).
//!   * ORDERFLOW CONFIRM (the separator): at/just after the sweep, top-N depth
//!     imbalance ABSORBED back toward the hit side => reversal likely; STAYS
//!     pulled => continuation likely. Used as a no-lookahead gate (read at the
//!     confirm-window-end <= entry). Reported with AND without, knob-bite shown.
//!
//! Deterministic + no-lookahead (only `<= now` state is read). PRICE = HL top-N
//! microprice (what we would actually trade against). Reuses `load_window` +
//! `forge_book::OrderBook` + the top-N microprice + imbalance()/top_depth() -
//! sacred core untouched.
//!
//! ```text
//! sweepscope --root /root/chd/fresh/ticks --coin ETH
//!   --dates 2025-11-04,2025-11-20 --hours all
//!   --lookbacks 15m,30m,60m --range-max 30,50,80 --sweep-margin 5,10,20
//!   --cooldown 30m --forward 90m --sample 1s --top 5
//!   --class-thr 20 --rev-target 40 --rev-stop 15 --cont-target 60 --cont-stop 20
//!   --confirm-imb 0.10 --confirm-trend-max 0.0 --confirm-window 60s
//!   --maker-fee 1.5 --dump rows.csv
//! ```

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitCode;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, Side, UnixNanos};
use forgelag::{load_window, FeedConfig, LagEvent, LagKind, Role};

// ----------------------------------------------------------------------------
// fee model (realistic HL): maker entry ~1.5bps, taker ~4.5bps each side.
// ----------------------------------------------------------------------------

/// Full taker round-trip fee (taker in + taker out) in bps.
const TAKER_RT_FEE_BPS: f64 = 9.0;
/// Taker exit fee (the maker-entry reversal exits by crossing).
const TAKER_EXIT_FEE_BPS: f64 = 4.5;

// ----------------------------------------------------------------------------
// small parse / math helpers (mirroring oiscope / gapscope conventions)
// ----------------------------------------------------------------------------

/// Parse a duration like `30m`, `90m`, `60s`, `1s`, `500ms` into ns.
fn parse_dur(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult): (&str, u64) = if let Some(p) = s.strip_suffix("ms") { (p, 1_000_000) }
        else if let Some(p) = s.strip_suffix("us") { (p, 1_000) }
        else if let Some(p) = s.strip_suffix("ns") { (p, 1) }
        else if let Some(p) = s.strip_suffix('h') { (p, 3_600_000_000_000) }
        else if let Some(p) = s.strip_suffix('m') { (p, 60_000_000_000) }
        else if let Some(p) = s.strip_suffix('s') { (p, 1_000_000_000) }
        else { (s, 1) };
    let v: f64 = num.trim().parse().map_err(|e| format!("bad duration `{s}`: {e}"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("bad duration `{s}`"));
    }
    Ok((v * mult as f64) as u64)
}

/// Parse a comma list of f64.
fn parse_f64_list(s: &str) -> Result<Vec<f64>, String> {
    s.split(',')
        .map(|x| x.trim().parse::<f64>().map_err(|e| format!("bad number `{x}`: {e}")))
        .collect()
}

/// Parse a comma list of durations (ns).
fn parse_dur_list(s: &str) -> Result<Vec<u64>, String> {
    s.split(',').map(|x| parse_dur(x.trim())).collect()
}

/// Size-weighted top-N microprice, copied from oiscope / `FairValueOracle::micro`.
#[must_use]
fn micro(book: &OrderBook, top_n: usize) -> Option<f64> {
    let (bb, _) = book.best_bid()?;
    let (ba, _) = book.best_ask()?;
    let bb = bb.to_f64();
    let ba = ba.to_f64();
    let bq: f64 = book.bids_iter().take(top_n).map(|(_, q)| q.raw() as f64).sum();
    let aq: f64 = book.asks_iter().take(top_n).map(|(_, q)| q.raw() as f64).sum();
    let tot = bq + aq;
    if tot <= 0.0 {
        return Some((bb + ba) / 2.0);
    }
    Some((bb * aq + ba * bq) / tot)
}

/// Mean of a slice (0.0 for empty).
fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

/// Median of a slice (sorts a copy; 0.0 for empty).
#[must_use]
fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

/// One-sample t-stat of a series against zero. 0.0 when fewer than two points or
/// zero variance (honest: no significance).
#[must_use]
fn tstat(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 2 {
        return 0.0;
    }
    let m = mean(v);
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n as f64 - 1.0);
    let sd = var.sqrt();
    if sd <= 0.0 {
        return 0.0;
    }
    m / (sd / (n as f64).sqrt())
}

/// Top-N depth imbalance in [-1, 1]: `(bid - ask) / (bid + ask)`. Positive = more
/// bid depth (book leans up). Copied from oiscope/gapscope so the tools agree.
#[must_use]
fn imbalance(bid_depth: f64, ask_depth: f64) -> f64 {
    let t = bid_depth + ask_depth;
    if t <= 0.0 { 0.0 } else { (bid_depth - ask_depth) / t }
}

/// Top-N (bid_depth, ask_depth) in base units. Copied from oiscope/gapscope.
#[must_use]
fn top_depth(book: &OrderBook, top_n: usize) -> (f64, f64) {
    let bid: f64 = book.bids_iter().take(top_n).map(|(_, q)| q.to_f64()).sum();
    let ask: f64 = book.asks_iter().take(top_n).map(|(_, q)| q.to_f64()).sum();
    (bid, ask)
}

// ----------------------------------------------------------------------------
// PURE, TESTED detection + classification + trade primitives
// ----------------------------------------------------------------------------

/// Width of a range in bps: `(hi-lo)/mid*1e4`, mid = midpoint. 0.0 for a
/// degenerate / non-positive range.
#[must_use]
fn range_width_bps(lo: f64, hi: f64) -> f64 {
    let mid = (lo + hi) / 2.0;
    if mid <= 0.0 || hi < lo {
        return 0.0;
    }
    (hi - lo) / mid * 10_000.0
}

/// SWEEP test: does microprice `m` poke BEYOND the established range `[lo,hi]` by
/// at least `margin_bps`? Returns `Some(true)` for an UP sweep (above hi),
/// `Some(false)` for a DOWN sweep (below lo), `None` for no sweep. Pure.
#[must_use]
fn detect_sweep(m: f64, lo: f64, hi: f64, margin_bps: f64) -> Option<bool> {
    if lo <= 0.0 || hi <= 0.0 {
        return None;
    }
    let up = hi * (1.0 + margin_bps / 10_000.0);
    let dn = lo * (1.0 - margin_bps / 10_000.0);
    if m >= up {
        Some(true)
    } else if m <= dn {
        Some(false)
    } else {
        None
    }
}

/// ORDERFLOW CONFIRM - ABSORPTION (reversal likely). A sweep was HIT on one side
/// (UP sweep consumed ASKS; DOWN sweep consumed BIDS). Absorption = liquidity
/// HOLDS / returns on the hit side, so top-N imbalance shifts back TOWARD the hit
/// side over the confirm window. UP sweep hit the ASK -> a revert is supported
/// when imbalance FALLS (toward ask); DOWN sweep hit the BID -> when imbalance
/// RISES (toward bid). Returns true when the toward-hit-side shift >= `min_shift`.
/// (Mirror of oiscope's confirm_revert; `dir_down` = the BID was hit = DOWN sweep.)
/// Pure.
#[must_use]
fn confirm_absorption(imb_start: f64, imb_end: f64, sweep_down: bool, min_shift: f64) -> bool {
    let toward_hit = if sweep_down { imb_end - imb_start } else { imb_start - imb_end };
    toward_hit >= min_shift
}

/// ORDERFLOW CONFIRM - FOLLOW-THROUGH (continuation likely). The mirror of
/// absorption: liquidity STAYS PULLED on the hit side (it does NOT return), so
/// the toward-hit-side imbalance shift is <= `max_return` (default 0 = no
/// meaningful return). Pure.
#[must_use]
fn confirm_followthrough(imb_start: f64, imb_end: f64, sweep_down: bool, max_return: f64) -> bool {
    let toward_hit = if sweep_down { imb_end - imb_start } else { imb_start - imb_end };
    toward_hit <= max_return
}

/// The classified forward outcome of a sweep.
#[derive(Clone, Copy, Debug, Default)]
struct Outcome {
    /// 1 = CONTINUATION, -1 = REVERSAL, 0 = NEITHER (within the horizon).
    class: i8,
    /// Max extension in the SWEEP direction from the entry (bps) over the path.
    cont_ext_bps: f64,
    /// Max move BACK against the sweep from the entry (bps) over the path - the
    /// reversal magnitude (how far it snapped back through / past the range).
    rev_back_bps: f64,
    /// ns from the fire to the first decision touch (None if NEITHER).
    decision_ns: Option<u64>,
}

/// Classify the forward `path` of a sweep (NO-lookahead, first-touch). `entry_px`
/// = microprice at the sweep poke (the extreme at detection); `edge` = the broken
/// range edge (hi for an UP sweep, lo for a DOWN sweep); `dir_up` = UP sweep;
/// `class_thr_bps` = how far PAST the entry, in the sweep direction, counts as a
/// real CONTINUATION. CONTINUATION fires if the path extends >= class_thr beyond
/// entry BEFORE it returns through the edge (re-enters the range); REVERSAL fires
/// if it re-enters the range first. Magnitudes are the max excursions over the
/// FULL path each way (the class is decided by which TOUCHED first). Pure.
#[must_use]
fn classify_outcome(path: &[(u64, f64)], fire_ts: u64, entry_px: f64, edge: f64, dir_up: bool, class_thr_bps: f64) -> Outcome {
    if path.is_empty() || entry_px <= 0.0 {
        return Outcome::default();
    }
    let s = if dir_up { 1.0 } else { -1.0 };
    let mut max_ext = 0.0f64;
    let mut max_back = 0.0f64;
    let mut class = 0i8;
    let mut decision_ns = None;
    for &(ts, px) in path {
        let ext = s * (px - entry_px) / entry_px * 10_000.0;
        if ext > max_ext {
            max_ext = ext;
        }
        if -ext > max_back {
            max_back = -ext;
        }
        if class == 0 {
            // CONTINUATION: extended class_thr beyond the entry in the sweep dir.
            if class_thr_bps > 0.0 && ext >= class_thr_bps {
                class = 1;
                decision_ns = Some(ts.saturating_sub(fire_ts));
            } else {
                // REVERSAL: re-entered the range (crossed back through the edge).
                let reentered = if dir_up { px <= edge } else { px >= edge };
                if reentered {
                    class = -1;
                    decision_ns = Some(ts.saturating_sub(fire_ts));
                }
            }
        }
    }
    Outcome { class, cont_ext_bps: max_ext, rev_back_bps: max_back, decision_ns }
}

/// Honest first-touch TAKER trade over a forward `path`. Enter at the first
/// sample at/after `entry_ts` (taker, marketable); `long` = buy side. Exit by the
/// FIRST of: `target_bps` favourable, `stop_bps` adverse, or `hold_ns` elapsed
/// (mark-to-market). Returns the signed bps captured (losers/run-overs INCLUDED),
/// or None if the path ended before the entry. Pure.
#[must_use]
fn first_touch(path: &[(u64, f64)], entry_ts: u64, long: bool, target_bps: f64, stop_bps: f64, hold_ns: u64) -> Option<f64> {
    let start = path.iter().position(|&(ts, _)| ts >= entry_ts)?;
    let entry_px = path[start].1;
    let entry_t = path[start].0;
    if entry_px <= 0.0 {
        return None;
    }
    let s = if long { 1.0 } else { -1.0 };
    let mut last = 0.0f64;
    for &(ts, px) in &path[start..] {
        let signed = s * (px - entry_px) / entry_px * 10_000.0;
        last = signed;
        if target_bps > 0.0 && signed >= target_bps {
            return Some(signed);
        }
        if stop_bps > 0.0 && signed <= -stop_bps {
            return Some(signed);
        }
        if ts.saturating_sub(entry_t) >= hold_ns {
            return Some(signed);
        }
    }
    Some(last)
}

/// Honest MAKER-in-path REVERSAL entry: rest a limit AT the swept range `edge`
/// (a SELL at hi for an UP sweep, a BUY at lo for a DOWN sweep). It FILLS only
/// when the HL tape actually TRADES THROUGH the edge in the filling direction at/
/// after `arm_ts` (UP: an aggressive BUY prints >= edge as the sweep pokes up;
/// DOWN: an aggressive SELL prints <= edge) - no free fill, no lookahead. The fill
/// price is the maker LIMIT (`edge`). Once filled we are AGAINST the sweep (short
/// for UP, long for DOWN) and exit by the SAME first-touch rule as the taker
/// (target/stop/hold) - TRENDERS that fill and run over return a REAL loss.
///
/// `hold_gate = Some((confirm_end_ts, confirm_ok))` wires the orderflow confirm as
/// a no-lookahead HOLD gate: if absorption did NOT confirm by the window-end, a
/// still-resting order is CANCELLED (None) and an already-filled position is
/// FLATTENED at the window-end. Returns (fill_ts, signed_bps) or None. Pure.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn simulate_maker_reversal(
    path: &[(u64, f64)],
    trades: &[(u64, f64, bool, f64)],
    arm_ts: u64,
    edge: f64,
    dir_up: bool,
    target_bps: f64,
    stop_bps: f64,
    hold_ns: u64,
    hold_gate: Option<(u64, bool)>,
) -> Option<(u64, f64)> {
    if edge <= 0.0 || path.is_empty() {
        return None;
    }
    // FILL: first aggressive print that trades THROUGH the resting edge at/after arm.
    let mut fill_ts: Option<u64> = None;
    for &(ts, px, buy, _q) in trades {
        if ts < arm_ts {
            continue;
        }
        let through = if dir_up { buy && px >= edge } else { !buy && px <= edge };
        if through {
            fill_ts = Some(ts);
            break;
        }
    }
    let fill_ts = fill_ts?;
    // HOLD-GATE: bail orders the confirm did not support by the window-end.
    let mut force_exit_ts: Option<u64> = None;
    if let Some((ce, ok)) = hold_gate {
        if !ok {
            if fill_ts > ce {
                return None; // still resting at the confirm read -> CANCELLED.
            }
            force_exit_ts = Some(ce); // filled before the cut -> flatten at the read.
        }
    }
    // EXIT: reversal position - short for UP sweep, long for DOWN. Entry at `edge`.
    let s = if dir_up { -1.0 } else { 1.0 };
    let mut last = 0.0f64;
    let mut started = false;
    for &(ts, px) in path {
        if ts < fill_ts {
            continue;
        }
        started = true;
        let signed = s * (px - edge) / edge * 10_000.0;
        last = signed;
        if target_bps > 0.0 && signed >= target_bps {
            return Some((fill_ts, signed));
        }
        if stop_bps > 0.0 && signed <= -stop_bps {
            return Some((fill_ts, signed));
        }
        if let Some(fe) = force_exit_ts {
            if ts >= fe {
                return Some((fill_ts, signed));
            }
        }
        if ts.saturating_sub(fill_ts) >= hold_ns {
            return Some((fill_ts, signed));
        }
    }
    if started { Some((fill_ts, last)) } else { Some((fill_ts, 0.0)) }
}
// ----------------------------------------------------------------------------
// NEW SEPARATOR: AGGRESSIVE TRADE FLOW vs PRICE IMPACT (the flow-vs-impact read)
// ----------------------------------------------------------------------------
// The prior depth-imbalance confirm FAILED to separate reversal from continuation
// (P(rev|confirm) ~= P(rev|no-confirm) = random). This is a DIFFERENT signal built
// from the HL AGGRESSIVE TRADES (not resting depth), over a no-lookahead window
// [fire, confirm-window-end] (every read <= entry). The trader's 3-behaviour
// taxonomy collapses to flow-vs-impact:
//   1. ABSORBED  - forced (sweep-dir) volume HIGH, price barely moves -> REVERSAL.
//   2. EXHAUSTED - late-half forced-volume RATE fades vs early half  -> REVERSAL.
//   3. EASY-PUSH - price moves FAR on LITTLE volume (vacuum)         -> CONTINUATION.

/// Scalar flow-vs-impact metrics over the measurement window. All pure.
#[derive(Clone, Copy, Debug, Default)]
struct FlowMetrics {
    /// Total ABSOLUTE aggressive HL volume in the window (base units).
    vol_abs: f64,
    /// Total SIGNED aggressive volume (buy +, sell -).
    vol_signed: f64,
    /// Aggressive volume in the SWEEP DIRECTION (the forced flow): UP sweep =
    /// aggressive buys, DOWN sweep = aggressive sells.
    vol_dir: f64,
    /// |microprice move| fire -> window-end, bps (the displacement).
    disp_bps: f64,
    /// Max extension in the sweep direction over the window, bps.
    max_ext_bps: f64,
    /// ABSORPTION = forced volume / displacement. HIGH = absorbed (behaviour 1).
    absorption_ratio: f64,
    /// IMPACT = displacement / forced volume. HIGH = easy push (behaviour 3).
    impact_ratio: f64,
    /// Forced volume in the EARLY half of the window.
    early_dir_vol: f64,
    /// Forced volume in the LATE half of the window.
    late_dir_vol: f64,
    /// late/early forced-volume rate ratio. LOW = exhausting (behaviour 2).
    decel_ratio: f64,
}

/// Compute the flow-vs-impact metrics over the NO-LOOKAHEAD window
/// `[fire_ts, window_end_ts]` (window_end = confirm-window-end = entry anchor, so
/// every read is <= entry). `trades` = (ts, px, buy_aggr, qty); `path` = the
/// microprice samples; `dir_up` = sweep direction; `fire_px` = microprice at the
/// poke. Trades/samples outside the window are ignored (no lookahead). Pure.
#[must_use]
fn compute_flow(
    trades: &[(u64, f64, bool, f64)],
    path: &[(u64, f64)],
    fire_ts: u64,
    window_end_ts: u64,
    fire_px: f64,
    dir_up: bool,
) -> FlowMetrics {
    let eps = 1e-9;
    let s = if dir_up { 1.0 } else { -1.0 };
    let mid_ts = fire_ts.saturating_add(window_end_ts.saturating_sub(fire_ts) / 2);
    let mut m = FlowMetrics::default();
    for &(ts, _px, buy, q) in trades {
        if ts < fire_ts || ts > window_end_ts {
            continue;
        }
        let q = if q.is_finite() && q > 0.0 { q } else { 0.0 };
        m.vol_abs += q;
        m.vol_signed += if buy { q } else { -q };
        // forced flow = aggression in the sweep direction.
        let dir_q = if dir_up == buy { q } else { 0.0 };
        m.vol_dir += dir_q;
        if ts <= mid_ts {
            m.early_dir_vol += dir_q;
        } else {
            m.late_dir_vol += dir_q;
        }
    }
    // displacement + max extension from the microprice path within the window.
    let mut end_px = fire_px;
    if fire_px > 0.0 {
        for &(ts, px) in path {
            if ts < fire_ts || ts > window_end_ts {
                continue;
            }
            end_px = px;
            let ext = s * (px - fire_px) / fire_px * 10_000.0;
            if ext > m.max_ext_bps {
                m.max_ext_bps = ext;
            }
        }
        m.disp_bps = ((end_px - fire_px) / fire_px * 10_000.0).abs();
    }
    m.absorption_ratio = m.vol_dir / m.disp_bps.max(eps);
    m.impact_ratio = m.disp_bps / m.vol_dir.max(eps);
    m.decel_ratio = m.late_dir_vol / m.early_dir_vol.max(eps);
    m
}

/// ABSORPTION confirm (behaviour 1 -> predicts REVERSAL): forced volume HIGH vs
/// the displacement it produced. True when absorption_ratio >= `absorb_min`
/// (requires real forced flow). Pure.
#[must_use]
fn confirm_absorb_flow(m: &FlowMetrics, absorb_min: f64) -> bool {
    m.vol_dir > 0.0 && m.absorption_ratio >= absorb_min
}

/// EXHAUSTION confirm (behaviour 2 -> predicts REVERSAL): the forced-volume RATE
/// fades into the late half. True when late-half forced volume <= `decel_frac` *
/// early-half (decel_ratio <= decel_frac), with real early flow to fade. Pure.
#[must_use]
fn confirm_exhaust_flow(m: &FlowMetrics, decel_frac: f64) -> bool {
    m.early_dir_vol > 0.0 && m.late_dir_vol <= decel_frac * m.early_dir_vol
}

/// EASY-PUSH confirm (behaviour 3 -> predicts CONTINUATION): price moved FAR on
/// LITTLE volume. True when impact_ratio >= `push_min` (requires real move). Pure.
#[must_use]
fn confirm_push_flow(m: &FlowMetrics, push_min: f64) -> bool {
    m.disp_bps > 0.0 && m.impact_ratio >= push_min
}

/// The trader's 3-state flow taxonomy, keyed on the MASTER VARIABLE
/// (price-impact-per-unit-forced-volume = `impact_ratio`) plus the flow-decay
/// (exhaustion). All inputs are window metrics (read <= entry) = no-lookahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlowState {
    /// LOW impact-per-volume WITH real sustained forced flow: a wall ate the
    /// flow (lots of volume, little displacement) -> predicts REVERSAL.
    Absorbed,
    /// Forced flow decelerates toward zero (late << early): the push is spent
    /// and price stalls -> predicts REVERSAL.
    Exhausted,
    /// HIGH impact-per-volume: price ran far on little volume into thin/pulled
    /// liquidity -> predicts CONTINUATION.
    Continuation,
    /// None of the three signatures fired.
    Unclassified,
}

/// Assign the 3-state flow outcome (pure). CONTINUATION (clean easy-push) is
/// tested first, then ABSORBED (low impact-per-vol with real forced flow), then
/// EXHAUSTED (decelerating forced flow). `impact_lo`/`impact_hi` are the two cuts
/// on the master variable; `flow_decay` is the late/early forced-volume ceiling
/// for exhaustion; `absorb_minvol` requires real forced flow for an absorption.
#[must_use]
fn assign_flow_state(m: &FlowMetrics, impact_lo: f64, impact_hi: f64, flow_decay: f64, absorb_minvol: f64) -> FlowState {
    if m.disp_bps > 0.0 && m.vol_dir > 0.0 && m.impact_ratio >= impact_hi {
        return FlowState::Continuation;
    }
    if m.vol_dir > absorb_minvol && m.impact_ratio <= impact_lo {
        return FlowState::Absorbed;
    }
    if m.early_dir_vol > 0.0 && m.decel_ratio <= flow_decay {
        return FlowState::Exhausted;
    }
    FlowState::Unclassified
}

/// P(target_class | signal) vs P(target_class | NOT signal) over decided sweeps,
/// returned as (p_sig%, n_sig, p_not%, n_not). The KEY separation primitive
/// (mirrors the depth-imbalance separation table). Pure + tested.
#[must_use]
fn separation(items: &[(bool, i8)], target: i8) -> (f64, usize, f64, usize) {
    let (mut nw, mut tw, mut nn, mut tn) = (0usize, 0usize, 0usize, 0usize);
    for &(sig, cls) in items {
        if sig {
            nw += 1;
            if cls == target {
                tw += 1;
            }
        } else {
            nn += 1;
            if cls == target {
                tn += 1;
            }
        }
    }
    let pw = if nw > 0 { tw as f64 / nw as f64 * 100.0 } else { 0.0 };
    let pn = if nn > 0 { tn as f64 / nn as f64 * 100.0 } else { 0.0 };
    (pw, nw, pn, nn)
}

/// Percentile (nearest-rank on a sorted copy) of a slice; 0.0 for empty. Pure.
#[must_use]
fn percentile(v: &[f64], pct: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (pct / 100.0 * (s.len() as f64 - 1.0)).round() as usize;
    s[rank.min(s.len() - 1)]
}

// ----------------------------------------------------------------------------
// configuration + per-sweep record
// ----------------------------------------------------------------------------

#[derive(Clone)]
struct Cfg {
    root: PathBuf,
    coin: String,
    hours: Vec<String>,
    cooldown_ns: u64,
    forward_ns: u64,
    sample_ns: u64,
    flow_window_ns: u64,
    top_n: usize,
    class_thr_bps: f64,
    rev_target_bps: f64,
    rev_stop_bps: f64,
    cont_target_bps: f64,
    cont_stop_bps: f64,
    hold_ns: u64,
    confirm_ns: u64,
    confirm_imb: f64,
    confirm_trend_max: f64,
    maker_fee_bps: f64,
    // NEW flow-vs-impact separator (default OFF / additive; output byte-preserved).
    flow_on: bool,
    absorb_gate: bool,
    exhaust_gate: bool,
    push_gate: bool,
    absorb_min: f64,
    exhaust_decel: f64,
    push_min: f64,
    // 3-state master-variable model (the trader's flow-vs-impact taxonomy).
    impact_lo: f64,
    impact_hi: f64,
    flow_decay: f64,
    absorb_minvol: f64,
    state_gate: bool,
    min_oidrop: f64,
    // OPTION A/B dynamic reversal exits (default OFF; additive, output byte-preserved).
    dyn_stop: bool,
    stop_buffer_bps: f64,
    rr_mult: f64,
    flow_exit: bool,
    flow_exit_win_ns: u64,
    flow_exit_k: f64,
    maker_rev: bool,
}

/// One detected sweep and its full characterisation (scalars only; the forward
/// path is transient and dropped after finalisation).
#[derive(Clone)]
struct SweepStat {
    day: String,
    l_ns: u64,
    range_max: f64,
    sweep_margin: f64,
    fire_ts: u64,
    dir_up: bool,
    range_lo: f64,
    range_hi: f64,
    range_width_bps: f64,
    mid: f64,
    entry_px: f64,
    /// OI-drop% over the lookback window at the sweep (recorded, NOT gating).
    oi_drop_pct: f64,
    /// Net signed aggressive HL flow over the flow window at the sweep (recorded).
    net_flow: f64,
    /// Forward outcome classification + magnitudes.
    class: i8,
    cont_ext_bps: f64,
    rev_back_bps: f64,
    decision_ns: Option<u64>,
    /// Orderflow confirm reads (no-lookahead: fire + confirm-window-end).
    imb_start: f64,
    imb_end: f64,
    /// Absorption confirmed (reversal-supporting) within the confirm window.
    rev_confirm: bool,
    /// Follow-through confirmed (continuation-supporting) within the window.
    cont_confirm: bool,
    /// REVERSAL trade, TAKER entry at the swept extreme (signed bps; run-overs in).
    rev_take: Option<f64>,
    /// REVERSAL trade, MAKER entry resting at the swept edge, UNGATED.
    rev_make: Option<f64>,
    rev_make_filled: bool,
    rev_make_fill_ns: Option<u64>,
    /// REVERSAL MAKER, GATED by the absorption confirm (hold-gate, no-lookahead).
    rev_make_gated: Option<f64>,
    rev_make_gated_filled: bool,
    /// CONTINUATION trade, TAKER entry with the break (signed bps; run-overs in).
    cont_take: Option<f64>,
    // ---- NEW flow-vs-impact metrics (recorded REGARDLESS of gating) ----
    flow: FlowMetrics,
    absorb_confirm: bool,
    exhaust_confirm: bool,
    push_confirm: bool,
    /// REVERSAL MAKER gated by the ABSORPTION-flow confirm (hold-gate, no-lookahead).
    rev_make_absorb: Option<f64>,
    rev_make_absorb_filled: bool,
    /// REVERSAL MAKER gated by the EXHAUSTION-flow confirm.
    rev_make_exhaust: Option<f64>,
    rev_make_exhaust_filled: bool,
    // ---- NEW 3-state (master-variable) assignment + depth-ahead corroboration ----
    /// 3-state assignment over the confirm window (read <= entry).
    state_absorbed: bool,
    state_exhausted: bool,
    state_continuation: bool,
    /// forced volume / depth-ahead drop over the window. HIGH = trades CONSUMED
    /// the resting wall (absorption); LOW = the wall was PULLED/cancelled (vacuum).
    consumed_frac: f64,
    /// REVERSAL MAKER gated by the 3-state (ABSORBED or EXHAUSTED), hold-gate.
    rev_make_state: Option<f64>,
    rev_make_state_filled: bool,
    // OPTION A/B dynamic-exit reversal taker variants (None unless the flag is on).
    rev_take_struct: Option<f64>,
    struct_stop_bps: f64,
    rev_take_flowexit: Option<f64>,
    // OPTION A maker (fee escape): structural-stop reversal MAKER, ungated + EXH-gated.
    rev_make_struct_all: Option<f64>,
    rev_make_struct_all_filled: bool,
    rev_make_struct: Option<f64>,
    rev_make_struct_filled: bool,
}

/// Pending sweep being collected over its forward window.
struct Pending {
    fire_ts: u64,
    dir_up: bool,
    range_lo: f64,
    range_hi: f64,
    range_width_bps: f64,
    mid: f64,
    entry_px: f64,
    oi_drop_pct: f64,
    net_flow: f64,
    end_ts: u64,
    imb_start: f64,
    imb_end: Option<f64>,
    dep_ahead_start: f64,
    dep_ahead_end: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Armed,
    Collecting,
}

/// Replay one day for ONE (lookback L, range-max, sweep-margin) triple; return
/// its sweeps. Deterministic + no-lookahead: book/OI/flow folded forward event by
/// event, only `<= now` state read. The sideways range is evaluated at the sample
/// cadence and the poke is tested against the range ESTABLISHED BEFORE this tick
/// (the poke is never folded into its own range = no lookahead).
fn scan_day(cfg: &Cfg, day: &str, evs: &[LagEvent], l_ns: u64, range_max: f64, sweep_margin: f64) -> Vec<SweepStat> {
    let mut book = OrderBook::with_max_levels(64);
    let mut oi_now = 0.0f64;
    let mut micro_now: Option<f64> = None;

    // sideways-range window buffer at sample cadence: (ts, micro, oi).
    let mut win: VecDeque<(u64, f64, f64)> = VecDeque::new();
    let mut last_sample = 0u64;
    // signed HL aggressive trade flow over the flow window: (ts, signed_qty).
    let mut flow: VecDeque<(u64, f64)> = VecDeque::new();

    let mut state = State::Armed;
    let mut blocked_until = 0u64;
    let mut cur = Pending {
        fire_ts: 0, dir_up: false, range_lo: 0.0, range_hi: 0.0, range_width_bps: 0.0,
        mid: 0.0, entry_px: 0.0, oi_drop_pct: 0.0, net_flow: 0.0, end_ts: 0,
        imb_start: 0.0, imb_end: None,
        dep_ahead_start: 0.0, dep_ahead_end: None,
    };
    let mut cur_path: Vec<(u64, f64)> = Vec::new();
    let mut cur_trades: Vec<(u64, f64, bool, f64)> = Vec::new();
    let mut out: Vec<SweepStat> = Vec::new();

    for ev in evs {
        let now = ev.local_ts;
        // ---- fold the event (sacred ordering) ----
        match (ev.role, ev.kind) {
            (Role::Exec, LagKind::BookDelta) => {
                if let Ok(e) = Event::new(
                    EventKind::BookDelta,
                    UnixNanos::new(ev.exch_ts),
                    UnixNanos::new(ev.local_ts),
                    ev.side,
                    ev.price,
                    ev.qty,
                    0,
                ) {
                    if book.apply(&e).is_ok() {
                        if let Some(m) = micro(&book, cfg.top_n) {
                            micro_now = Some(m);
                        }
                    }
                }
            }
            (Role::Exec, LagKind::Trade) => {
                if let Some(aggr) = ev.side {
                    let q = ev.qty.to_f64();
                    let signed = if aggr == Side::Bid { q } else { -q };
                    flow.push_back((now, signed));
                    if matches!(state, State::Collecting) {
                        cur_trades.push((now, ev.price.to_f64(), aggr == Side::Bid, q));
                    }
                }
            }
            (_, LagKind::OpenInterest) => oi_now = ev.aux,
            _ => {}
        }

        // evict old aggressive flow.
        let flo = now.saturating_sub(cfg.flow_window_ns);
        while let Some(&(ts, _)) = flow.front() {
            if ts < flo { flow.pop_front(); } else { break; }
        }

        match state {
            State::Collecting => {
                if let Some(m) = micro_now {
                    let push = match cur_path.last() {
                        Some(&(t, _)) => now.saturating_sub(t) >= cfg.sample_ns,
                        None => true,
                    };
                    if push {
                        cur_path.push((now, m));
                    }
                }
                if cur.imb_end.is_none() && now >= cur.fire_ts.saturating_add(cfg.confirm_ns) {
                    let (bd, ad) = top_depth(&book, cfg.top_n);
                    cur.imb_end = Some(imbalance(bd, ad));
                    cur.dep_ahead_end = Some(if cur.dir_up { ad } else { bd });
                }
                if now >= cur.end_ts {
                    // ensure the confirm read happened (short horizons guard).
                    if cur.imb_end.is_none() {
                        let (bd, ad) = top_depth(&book, cfg.top_n);
                        cur.imb_end = Some(imbalance(bd, ad));
                        cur.dep_ahead_end = Some(if cur.dir_up { ad } else { bd });
                    }
                    out.push(finalize(&cur, &cur_path, &cur_trades, cfg, day, l_ns, range_max, sweep_margin));
                    cur_path.clear();
                    cur_trades.clear();
                    state = State::Armed;
                }
            }
            State::Armed => {
                // sample-cadence range evaluation + buffer push.
                let due = win.is_empty() || now.saturating_sub(last_sample) >= cfg.sample_ns;
                if due {
                    if let Some(m) = micro_now {
                        // evict samples older than the lookback L.
                        let lo_ts = now.saturating_sub(l_ns);
                        while let Some(&(ts, _, _)) = win.front() {
                            if ts < lo_ts { win.pop_front(); } else { break; }
                        }
                        // test the poke against the range ESTABLISHED so far (no
                        // lookahead): the current tick is NOT yet in the window.
                        if now >= blocked_until && win.len() >= 2 {
                            let span = now.saturating_sub(win.front().map_or(now, |&(t, _, _)| t));
                            let lo = win.iter().map(|&(_, p, _)| p).fold(f64::INFINITY, f64::min);
                            let hi = win.iter().map(|&(_, p, _)| p).fold(f64::NEG_INFINITY, f64::max);
                            let width = range_width_bps(lo, hi);
                            let sideways = width > 0.0 && width <= range_max;
                            if span >= l_ns / 2 && sideways {
                                if let Some(up) = detect_sweep(m, lo, hi, sweep_margin) {
                                    let oi_start = win.front().map_or(0.0, |&(_, _, o)| o);
                                    let oi_drop = if oi_start > 0.0 { (oi_start - oi_now) / oi_start * 100.0 } else { 0.0 };
                                    let net: f64 = flow.iter().map(|&(_, q)| q).sum();
                                    let (bd, ad) = top_depth(&book, cfg.top_n);
                                    cur = Pending {
                                        fire_ts: now,
                                        dir_up: up,
                                        range_lo: lo,
                                        range_hi: hi,
                                        range_width_bps: width,
                                        mid: (lo + hi) / 2.0,
                                        entry_px: m,
                                        oi_drop_pct: oi_drop,
                                        net_flow: net,
                                        end_ts: now.saturating_add(cfg.forward_ns),
                                        imb_start: imbalance(bd, ad),
                                        imb_end: None,
                                        dep_ahead_start: if up { ad } else { bd },
                                        dep_ahead_end: None,
                                    };
                                    cur_path.clear();
                                    cur_trades.clear();
                                    cur_path.push((now, m));
                                    blocked_until = now.saturating_add(cfg.cooldown_ns);
                                    win.clear();
                                    last_sample = now;
                                    state = State::Collecting;
                                    continue;
                                }
                            }
                        }
                        // fold this tick into the window for future evaluations.
                        win.push_back((now, m, oi_now));
                        last_sample = now;
                    }
                }
            }
        }
    }
    // a sweep still collecting at end-of-day is dropped (incomplete horizon).
    out
}

/// Finalise a collected sweep into scalar statistics.
#[allow(clippy::too_many_arguments)]
fn finalize(cur: &Pending, path: &[(u64, f64)], trades: &[(u64, f64, bool, f64)], cfg: &Cfg, day: &str, l_ns: u64, range_max: f64, sweep_margin: f64) -> SweepStat {
    let dir_up = cur.dir_up;
    let sweep_down = !dir_up;
    let edge = if dir_up { cur.range_hi } else { cur.range_lo };
    // forward outcome classification (no-lookahead, first-touch).
    let oc = classify_outcome(path, cur.fire_ts, cur.entry_px, edge, dir_up, cfg.class_thr_bps);
    // orderflow confirm reads (imb_start at fire, imb_end at confirm-window-end).
    let imb_start = cur.imb_start;
    let imb_end = cur.imb_end.unwrap_or(imb_start);
    let rev_confirm = confirm_absorption(imb_start, imb_end, sweep_down, cfg.confirm_imb);
    let cont_confirm = confirm_followthrough(imb_start, imb_end, sweep_down, cfg.confirm_trend_max);
    // entry anchor for the TAKER trades = the confirm-window-end (we observe the
    // brief confirm window before entering; on a swing timescale this is slack and
    // keeps the confirm read <= entry = no-lookahead). Both taker trades share it.
    let anchor = cur.fire_ts.saturating_add(cfg.confirm_ns);
    // (A) REVERSAL taker: enter AGAINST the sweep (UP -> short, DOWN -> long).
    let rev_take = first_touch(path, anchor, sweep_down, cfg.rev_target_bps, cfg.rev_stop_bps, cfg.hold_ns);
    // (A) REVERSAL maker: rest a limit at the swept edge; fill on trade-through.
    let (rev_make, rev_make_filled, rev_make_fill_ns) =
        match simulate_maker_reversal(path, trades, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rev_stop_bps, cfg.hold_ns, None) {
            Some((fts, bps)) => (Some(bps), true, Some(fts.saturating_sub(cur.fire_ts))),
            None => (None, false, None),
        };
    // (A) REVERSAL maker GATED by absorption confirm (hold-gate, no-lookahead).
    let (rev_make_gated, rev_make_gated_filled) =
        match simulate_maker_reversal(path, trades, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rev_stop_bps, cfg.hold_ns, Some((anchor, rev_confirm))) {
            Some((_, bps)) => (Some(bps), true),
            None => (None, false),
        };
    // (B) CONTINUATION taker: enter WITH the break (UP -> long, DOWN -> short).
    let cont_take = first_touch(path, anchor, dir_up, cfg.cont_target_bps, cfg.cont_stop_bps, cfg.hold_ns);
    // ---- NEW flow-vs-impact metrics over the no-lookahead window [fire, anchor] ----
    let flow = compute_flow(trades, path, cur.fire_ts, anchor, cur.entry_px, dir_up);
    let absorb_confirm = confirm_absorb_flow(&flow, cfg.absorb_min);
    let exhaust_confirm = confirm_exhaust_flow(&flow, cfg.exhaust_decel);
    let push_confirm = confirm_push_flow(&flow, cfg.push_min);
    // REVERSAL maker gated by each flow confirm (hold-gate: cancel if unconfirmed
    // by the window-end, flatten an early fill at the read - no-lookahead).
    let (rev_make_absorb, rev_make_absorb_filled) =
        match simulate_maker_reversal(path, trades, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rev_stop_bps, cfg.hold_ns, Some((anchor, absorb_confirm))) {
            Some((_, bps)) => (Some(bps), true),
            None => (None, false),
        };
    let (rev_make_exhaust, rev_make_exhaust_filled) =
        match simulate_maker_reversal(path, trades, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rev_stop_bps, cfg.hold_ns, Some((anchor, exhaust_confirm))) {
            Some((_, bps)) => (Some(bps), true),
            None => (None, false),
        };
    // ---- 3-state master-variable assignment + depth-ahead corroboration ----
    let fstate = assign_flow_state(&flow, cfg.impact_lo, cfg.impact_hi, cfg.flow_decay, cfg.absorb_minvol);
    let state_absorbed = fstate == FlowState::Absorbed;
    let state_exhausted = fstate == FlowState::Exhausted;
    let state_continuation = fstate == FlowState::Continuation;
    // depth ahead of the sweep CONSUMED (traded through) vs PULLED (cancelled): the
    // forced volume relative to how much depth-ahead disappeared over the window.
    let dep_drop = (cur.dep_ahead_start - cur.dep_ahead_end.unwrap_or(cur.dep_ahead_start)).max(0.0);
    // CONSUMED share in [0,1] (rough corroboration): of the depth-ahead that
    // disappeared, how much aggressive trade volume traded through it. ~1 = trades
    // ATE the wall (consumed -> absorption); ~0 = the wall was PULLED (vacuum ->
    // continuation). NaN when there is no information (no drop AND no forced flow).
    let consumed_frac = if dep_drop > 1e-9 {
        (flow.vol_dir / dep_drop).min(1.0)
    } else if flow.vol_dir > 0.0 {
        1.0
    } else {
        f64::NAN
    };
    // REVERSAL maker gated by the 3-state (ABSORBED or EXHAUSTED = reversal-predicting),
    // wired as the no-lookahead hold-gate (cancel if unconfirmed by the window-end).
    let state_rev_ok = state_absorbed || state_exhausted;
    let (rev_make_state, rev_make_state_filled) =
        match simulate_maker_reversal(path, trades, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rev_stop_bps, cfg.hold_ns, Some((anchor, state_rev_ok))) {
            Some((_, bps)) => (Some(bps), true),
            None => (None, false),
        };
    // OPTION A/B (default OFF): dynamic-stop + flow-reaccel-exit REVERSAL taker.
    // our reversal side: UP sweep -> SHORT (long=false), DOWN sweep -> LONG (true).
    let rev_long = sweep_down;
    let (rev_take_struct, struct_stop_bps) = if cfg.dyn_stop {
        match first_touch_struct(path, cur.fire_ts, anchor, rev_long, cfg.rev_target_bps, cfg.rr_mult, cfg.stop_buffer_bps, cfg.hold_ns) {
            Some((bps, sd)) => (Some(bps), sd),
            None => (None, f64::NAN),
        }
    } else {
        (None, f64::NAN)
    };
    let rev_take_flowexit = if cfg.flow_exit {
        let base_rate = flow.vol_dir / cfg.confirm_ns.max(1) as f64;
        first_touch_flow_exit(path, trades, anchor, rev_long, dir_up, cfg.rev_target_bps, cfg.hold_ns, cfg.flow_exit_win_ns, base_rate, cfg.flow_exit_k)
    } else {
        None
    };
    // OPTION A maker (fee escape): structural-stop reversal MAKER, ungated + EXH-gated.
    let (rev_make_struct_all, rev_make_struct_all_filled) = if cfg.maker_rev {
        match simulate_maker_reversal_struct(path, trades, cur.fire_ts, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rr_mult, cfg.stop_buffer_bps, cfg.hold_ns, None) {
            Some((_, bps, _)) => (Some(bps), true),
            None => (None, false),
        }
    } else {
        (None, false)
    };
    let (rev_make_struct, rev_make_struct_filled) = if cfg.maker_rev {
        match simulate_maker_reversal_struct(path, trades, cur.fire_ts, cur.fire_ts, edge, dir_up, cfg.rev_target_bps, cfg.rr_mult, cfg.stop_buffer_bps, cfg.hold_ns, Some((anchor, state_exhausted))) {
            Some((_, bps, _)) => (Some(bps), true),
            None => (None, false),
        }
    } else {
        (None, false)
    };
    SweepStat {
        day: day.to_string(),
        l_ns,
        range_max,
        sweep_margin,
        fire_ts: cur.fire_ts,
        dir_up,
        range_lo: cur.range_lo,
        range_hi: cur.range_hi,
        range_width_bps: cur.range_width_bps,
        mid: cur.mid,
        entry_px: cur.entry_px,
        oi_drop_pct: cur.oi_drop_pct,
        net_flow: cur.net_flow,
        class: oc.class,
        cont_ext_bps: oc.cont_ext_bps,
        rev_back_bps: oc.rev_back_bps,
        decision_ns: oc.decision_ns,
        imb_start,
        imb_end,
        rev_confirm,
        cont_confirm,
        rev_take,
        rev_make,
        rev_make_filled,
        rev_make_fill_ns,
        rev_make_gated,
        rev_make_gated_filled,
        cont_take,
        flow,
        absorb_confirm,
        exhaust_confirm,
        push_confirm,
        rev_make_absorb,
        rev_make_absorb_filled,
        rev_make_exhaust,
        rev_make_exhaust_filled,
        state_absorbed,
        state_exhausted,
        state_continuation,
        consumed_frac,
        rev_make_state,
        rev_make_state_filled,
        rev_take_struct,
        struct_stop_bps,
        rev_take_flowexit,
        rev_make_struct_all,
        rev_make_struct_all_filled,
        rev_make_struct,
        rev_make_struct_filled,
    }
}

// ----------------------------------------------------------------------------
// aggregate reporting
// ----------------------------------------------------------------------------

fn fmt_ns(ns: u64) -> String {
    if ns >= 60_000_000_000 {
        format!("{:.0}m", ns as f64 / 60e9)
    } else if ns >= 1_000_000_000 {
        format!("{:.0}s", ns as f64 / 1e9)
    } else {
        format!("{}ms", ns / 1_000_000)
    }
}

/// Print one honest trade-stat line for a set of realised signed-bps captures.
/// `fee_bps` is the realistic round-trip fee subtracted to get NET.
fn report_trades(label: &str, caps: &[f64], fee_bps: f64) {
    if caps.is_empty() {
        println!("  {label:<28}  (no entries)");
        return;
    }
    let n = caps.len();
    let wins = caps.iter().filter(|&&x| x > 0.0).count();
    let g = mean(caps);
    let worst = caps.iter().copied().fold(f64::INFINITY, f64::min);
    let win_mag = {
        let w: Vec<f64> = caps.iter().copied().filter(|&x| x > 0.0).collect();
        mean(&w)
    };
    let loss_mag = {
        let l: Vec<f64> = caps.iter().copied().filter(|&x| x < 0.0).map(|x| -x).collect();
        mean(&l)
    };
    let rr = if loss_mag > 0.0 { win_mag / loss_mag } else { 0.0 };
    println!(
        "  {label:<28}  n={n:<4} win {:>3.0}%  GROSS {:+.1}bps  t {:+.2}  worst {:+.0}  RR {:.2}  NET(-{:.1}) {:+.1}bps",
        wins as f64 / n as f64 * 100.0, g, tstat(caps), worst, rr, fee_bps, g - fee_bps
    );
}

/// Captures (Some only) of a trade field across sweeps, optionally filtered.
fn caps_of<F: Fn(&SweepStat) -> Option<f64>>(ds: &[&SweepStat], f: F) -> Vec<f64> {
    ds.iter().filter_map(|s| f(s)).collect()
}

/// Compact one-line per-day summary.
fn report_day_line(day: &str, ds: &[&SweepStat], cfg: &Cfg) {
    let n = ds.len();
    if n == 0 {
        println!("  {day}   sweeps 0");
        return;
    }
    let cont = ds.iter().filter(|s| s.class == 1).count();
    let rev = ds.iter().filter(|s| s.class == -1).count();
    let mk = caps_of(ds, |s| if s.rev_make_filled { s.rev_make } else { None });
    let ct = caps_of(ds, |s| s.cont_take);
    println!(
        "  {day}   sweeps {n}  cont {cont} rev {rev}  rev-maker(net) {:+.1}  cont-taker(net) {:+.1}",
        if mk.is_empty() { 0.0 } else { mean(&mk) - cfg.maker_fee_bps - TAKER_EXIT_FEE_BPS },
        if ct.is_empty() { 0.0 } else { mean(&ct) - TAKER_RT_FEE_BPS }
    );
}

/// Full pooled report block for one (L, range-max, sweep-margin) triple.
#[allow(clippy::too_many_lines)]
fn report_pooled(label: &str, ds: &[&SweepStat], cfg: &Cfg, num_days: usize) {
    println!("\n---------- {label} ----------");
    let n = ds.len();
    let per_day = if num_days > 0 { n as f64 / num_days as f64 } else { 0.0 };
    println!("sweeps {n}   ({per_day:.1}/day over {num_days} days)");
    if n == 0 {
        println!("  (none detected - range-max too tight, sweep-margin too wide, or quiet regime)");
        return;
    }
    let up = ds.iter().filter(|s| s.dir_up).count();
    let width: Vec<f64> = ds.iter().map(|s| s.range_width_bps).collect();
    let oidrop: Vec<f64> = ds.iter().map(|s| s.oi_drop_pct).collect();
    let netflow: Vec<f64> = ds.iter().map(|s| s.net_flow).collect();
    println!("direction                     UP {up}  DOWN {}", n - up);
    println!(
        "range width / OI-drop / flow   width mean {:.1}bps   OI-drop mean {:+.2}%   net-flow mean {:+.3} (recorded, not gating)",
        mean(&width), mean(&oidrop), mean(&netflow)
    );

    // ----- (1) CONTINUATION vs REVERSAL split + magnitudes + times -----
    let cont = ds.iter().filter(|s| s.class == 1).count();
    let rev = ds.iter().filter(|s| s.class == -1).count();
    let neither = n - cont - rev;
    println!(
        "OUTCOME split                 CONTINUATION {}/{n} = {:.0}%   REVERSAL {}/{n} = {:.0}%   neither {:.0}%",
        cont, cont as f64 / n as f64 * 100.0,
        rev, rev as f64 / n as f64 * 100.0,
        neither as f64 / n as f64 * 100.0
    );
    let cont_ext: Vec<f64> = ds.iter().filter(|s| s.class == 1).map(|s| s.cont_ext_bps).collect();
    let cont_t: Vec<f64> = ds.iter().filter(|s| s.class == 1).filter_map(|s| s.decision_ns).map(|x| x as f64 / 1e9).collect();
    let rev_back: Vec<f64> = ds.iter().filter(|s| s.class == -1).map(|s| s.rev_back_bps).collect();
    let rev_t: Vec<f64> = ds.iter().filter(|s| s.class == -1).filter_map(|s| s.decision_ns).map(|x| x as f64 / 1e9).collect();
    println!(
        "  continuation magnitude      mean {:.0}bps  median {:.0}bps   time-to-decide median {:.0}s",
        mean(&cont_ext), median(&cont_ext), median(&cont_t)
    );
    println!(
        "  reversal magnitude          mean {:.0}bps  median {:.0}bps   time-to-decide median {:.0}s",
        mean(&rev_back), median(&rev_back), median(&rev_t)
    );

    // ----- (2) ORDERFLOW CONFIRM separation (does it split them ahead of time?) -----
    let abs_n = ds.iter().filter(|s| s.rev_confirm).count();
    let ft_n = ds.iter().filter(|s| s.cont_confirm).count();
    println!(
        "ORDERFLOW CONFIRM (no-lookahead, read at fire+{})  absorption(rev-supporting) {}/{n} = {:.0}%   follow-through(cont-supporting) {}/{n} = {:.0}%",
        fmt_ns(cfg.confirm_ns), abs_n, abs_n as f64 / n as f64 * 100.0, ft_n, ft_n as f64 / n as f64 * 100.0
    );
    // separation: P(reversal | absorption) vs P(reversal | no absorption).
    let decided: Vec<&&SweepStat> = ds.iter().filter(|s| s.class != 0).collect();
    if !decided.is_empty() {
        let with = decided.iter().filter(|s| s.rev_confirm).collect::<Vec<_>>();
        let without = decided.iter().filter(|s| !s.rev_confirm).collect::<Vec<_>>();
        let p_rev_with = if with.is_empty() { 0.0 } else { with.iter().filter(|s| s.class == -1).count() as f64 / with.len() as f64 * 100.0 };
        let p_rev_without = if without.is_empty() { 0.0 } else { without.iter().filter(|s| s.class == -1).count() as f64 / without.len() as f64 * 100.0 };
        println!(
            "  SEPARATION (of decided)     P(reversal | absorption) {:.0}%  (n={})   vs  P(reversal | no-absorption) {:.0}%  (n={})   <- knob-bite if these differ",
            p_rev_with, with.len(), p_rev_without, without.len()
        );
    }

    // ----- (3) the two honest trades, with + without the confirm gate -----
    let mk_fee = cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS;
    println!("(A) REVERSAL  (against the sweep: UP->short, DOWN->long)");
    let mk_filled = ds.iter().filter(|s| s.rev_make_filled).count();
    let mk_wait: Vec<f64> = ds.iter().filter_map(|s| s.rev_make_fill_ns).map(|x| x as f64 / 1e9).collect();
    println!(
        "    maker-entry @ swept edge: filled {mk_filled}/{n} = {:.0}%   median fill-wait {:.0}s   (the maker fee escape: ~{:.1}bps RT vs ~{:.0}bps taker)",
        mk_filled as f64 / n as f64 * 100.0, if mk_wait.is_empty() { 0.0 } else { median(&mk_wait) }, mk_fee, TAKER_RT_FEE_BPS
    );
    report_trades("rev-MAKER (all)", &caps_of(ds, |s| if s.rev_make_filled { s.rev_make } else { None }), mk_fee);
    report_trades("rev-MAKER (+absorb gate)", &caps_of(ds, |s| if s.rev_make_gated_filled { s.rev_make_gated } else { None }), mk_fee);
    report_trades("rev-TAKER (all)", &caps_of(ds, |s| s.rev_take), TAKER_RT_FEE_BPS);
    report_trades("rev-TAKER (+absorb gate)", &caps_of(ds, |s| if s.rev_confirm { s.rev_take } else { None }), TAKER_RT_FEE_BPS);
    println!("(B) CONTINUATION  (with the break: UP->long, DOWN->short; taker)");
    report_trades("cont-TAKER (all)", &caps_of(ds, |s| s.cont_take), TAKER_RT_FEE_BPS);
    report_trades("cont-TAKER (+follow gate)", &caps_of(ds, |s| if s.cont_confirm { s.cont_take } else { None }), TAKER_RT_FEE_BPS);

    // ----- OPTION A/B dynamic-exit REVERSAL variants (only when ON; additive) -----
    if cfg.dyn_stop || cfg.flow_exit || cfg.maker_rev {
        println!("(A2) DYNAMIC-EXIT REVERSAL (taker structural/flow-exit + maker fee-escape)");
        if cfg.dyn_stop {
            let sds: Vec<f64> = ds.iter().filter(|s| s.struct_stop_bps.is_finite()).map(|s| s.struct_stop_bps).collect();
            println!(
                "    structural stop dist: mean {:.0}bps  median {:.0}bps  (dynamic, beyond the wick + {:.0}bps buffer; rr {:.2})",
                if sds.is_empty() { 0.0 } else { mean(&sds) }, median(&sds), cfg.stop_buffer_bps, cfg.rr_mult
            );
            report_trades("rev-TAKER struct (all)", &caps_of(ds, |s| s.rev_take_struct), TAKER_RT_FEE_BPS);
            report_trades("rev-TAKER struct (+EXH)", &caps_of(ds, |s| if s.state_exhausted { s.rev_take_struct } else { None }), TAKER_RT_FEE_BPS);
            report_trades("rev-TAKER struct (+ABS|EXH)", &caps_of(ds, |s| if s.state_absorbed || s.state_exhausted { s.rev_take_struct } else { None }), TAKER_RT_FEE_BPS);
        }
        if cfg.flow_exit {
            report_trades("rev-TAKER flowexit (all)", &caps_of(ds, |s| s.rev_take_flowexit), TAKER_RT_FEE_BPS);
            report_trades("rev-TAKER flowexit (+EXH)", &caps_of(ds, |s| if s.state_exhausted { s.rev_take_flowexit } else { None }), TAKER_RT_FEE_BPS);
            report_trades("rev-TAKER flowexit (+ABS|EXH)", &caps_of(ds, |s| if s.state_absorbed || s.state_exhausted { s.rev_take_flowexit } else { None }), TAKER_RT_FEE_BPS);
        }
        if cfg.maker_rev {
            let mk_fee = cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS;
            let filled = ds.iter().filter(|s| s.rev_make_struct_all_filled).count();
            let gfilled = ds.iter().filter(|s| s.rev_make_struct_filled).count();
            println!(
                "    MAKER rest @ swept edge (fee escape ~{:.1}bps RT vs {:.0} taker): filled {}/{}  (EXH-gated kept {})",
                mk_fee, TAKER_RT_FEE_BPS, filled, ds.len(), gfilled
            );
            report_trades("rev-MAKER struct (all)", &caps_of(ds, |s| if s.rev_make_struct_all_filled { s.rev_make_struct_all } else { None }), mk_fee);
            report_trades("rev-MAKER struct (+EXH gate)", &caps_of(ds, |s| if s.rev_make_struct_filled { s.rev_make_struct } else { None }), mk_fee);
        }
    }

    // ----- NEW flow-vs-impact separator (only when ON; additive, byte-preserved) -----
    if cfg.flow_on {
        report_flow_separator(ds, cfg);
        // OI-DROP GATE (the liquidation-footprint filter): re-run the SAME separator
        // on ONLY the sweeps with a real OI-drop over the lookback, so we see the
        // setup WITH vs WITHOUT the liquidation trigger.
        if cfg.min_oidrop > 0.0 {
            let gated: Vec<&SweepStat> = ds.iter().copied().filter(|s| s.oi_drop_pct >= cfg.min_oidrop).collect();
            println!(
                "\n  ===== WITH OI-DROP GATE (oi_drop_pct >= {:.2}%):  {} of {} sweeps kept =====",
                cfg.min_oidrop, gated.len(), ds.len()
            );
            report_flow_separator(&gated, cfg);
        }
    }
}

/// Print one separation line: P(target|signal) vs P(target|NOT signal) + lift.
fn sep_print(decided: &[&SweepStat], pred: impl Fn(&SweepStat) -> bool, target: i8, tag: &str, thr: f64, pct: f64) {
    let items: Vec<(bool, i8)> = decided.iter().map(|&s| (pred(s), s.class)).collect();
    let (pw, nw, pn, nn) = separation(&items, target);
    let what = if target == -1 { "rev " } else { "cont" };
    println!(
        "    {tag} {thr:>10.4} (p{pct:>2.0})  P({what}|sig) {pw:>3.0}% n={nw:<4}  P({what}|!sig) {pn:>3.0}% n={nn:<4}  lift {:+.0}pp",
        pw - pn
    );
}

/// THE KEY MEASUREMENT: does aggressive-flow-vs-price-impact SEPARATE reversal vs
/// continuation AHEAD of time (read <= entry)? Reports base rates + the three
/// signals' conditional probabilities swept across self-calibrated percentile
/// thresholds (knob-bite: the gate must move the set AND, ideally, the outcome
/// probability). Then, when a gate flag is on, the honest net-of-fee trade lines.
#[allow(clippy::too_many_lines)]
fn report_flow_separator(ds: &[&SweepStat], cfg: &Cfg) {
    let decided: Vec<&SweepStat> = ds.iter().copied().filter(|s| s.class != 0).collect();
    let nd = decided.len();
    println!("FLOW-vs-IMPACT separator (aggressive trade flow vs price impact; NO depth-imbalance)");
    if nd < 10 {
        println!("  (too few decided sweeps for separation: n={nd})");
        return;
    }
    let base_rev = decided.iter().filter(|s| s.class == -1).count() as f64 / nd as f64 * 100.0;
    let base_cont = decided.iter().filter(|s| s.class == 1).count() as f64 / nd as f64 * 100.0;
    println!("  base rates (decided n={nd}):  REVERSAL {base_rev:.0}%   CONTINUATION {base_cont:.0}%");
    let abss: Vec<f64> = decided.iter().map(|s| s.flow.absorption_ratio).collect();
    let imps: Vec<f64> = decided.iter().map(|s| s.flow.impact_ratio).collect();
    let decs: Vec<f64> = decided.iter().filter(|s| s.flow.early_dir_vol > 0.0).map(|s| s.flow.decel_ratio).collect();
    let dirv: Vec<f64> = decided.iter().map(|s| s.flow.vol_dir).collect();
    let disp: Vec<f64> = decided.iter().map(|s| s.flow.disp_bps).collect();
    println!(
        "  metric medians:  forced-vol {:.3}  disp {:.2}bps  absorption {:.3}  impact {:.4}  decel {:.2}",
        median(&dirv), median(&disp), median(&abss), median(&imps), median(&decs)
    );
    // ---- 3-STATE @ CONFIGURED thresholds (the trader's dials): does each state
    // predict its outcome vs the BASE RATE? P(rev|ABSORBED), P(rev|EXHAUSTED),
    // P(cont|CONTINUATION). Depth-ahead consumed-vs-pulled reported as corroboration.
    let na = decided.iter().filter(|s| s.state_absorbed).count();
    let ne = decided.iter().filter(|s| s.state_exhausted).count();
    let nc = decided.iter().filter(|s| s.state_continuation).count();
    let nu = nd - na - ne - nc;
    println!(
        "  STATE ASSIGN @ impact-lo {:.4}/impact-hi {:.4}/flow-decay {:.3}:  ABSORBED {na}  EXHAUSTED {ne}  CONTINUATION {nc}  unclassified {nu}",
        cfg.impact_lo, cfg.impact_hi, cfg.flow_decay
    );
    let abs_items: Vec<(bool, i8)> = decided.iter().map(|s| (s.state_absorbed, s.class)).collect();
    let exh_items: Vec<(bool, i8)> = decided.iter().map(|s| (s.state_exhausted, s.class)).collect();
    let cont_items: Vec<(bool, i8)> = decided.iter().map(|s| (s.state_continuation, s.class)).collect();
    let (pa, _, pan, _) = separation(&abs_items, -1);
    let (pe, _, pen, _) = separation(&exh_items, -1);
    let (pc, _, pcn, _) = separation(&cont_items, 1);
    let cons_a = median(&decided.iter().filter(|s| s.state_absorbed && s.consumed_frac.is_finite()).map(|s| s.consumed_frac).collect::<Vec<_>>());
    let cons_c = median(&decided.iter().filter(|s| s.state_continuation && s.consumed_frac.is_finite()).map(|s| s.consumed_frac).collect::<Vec<_>>());
    println!("    ABSORBED      P(rev |state) {pa:>3.0}% (n={na:<4}) vs base {base_rev:>3.0}%   P(rev |!state) {pan:>3.0}%   depth-consumed med {cons_a:.2}");
    println!("    EXHAUSTED     P(rev |state) {pe:>3.0}% (n={ne:<4}) vs base {base_rev:>3.0}%   P(rev |!state) {pen:>3.0}%");
    println!("    CONTINUATION  P(cont|state) {pc:>3.0}% (n={nc:<4}) vs base {base_cont:>3.0}%   P(cont|!state) {pcn:>3.0}%   depth-consumed med {cons_c:.2}");

    // KNOB-BITE on the MASTER VARIABLE (impact-per-unit-forced-volume), swept BOTH
    // ways from the SAME dial: LOW impact = ABSORBED (predicts REVERSAL); HIGH impact
    // = EASY-PUSH (predicts CONTINUATION). This is the whole separation test.
    // 1. ABSORBED -> REVERSAL: LOW impact-per-vol (vol eaten, price stuck). Lower pctiles.
    println!("  [1] ABSORBED (low impact-per-vol) -> predicts REVERSAL  (gate: impact_ratio <= thr, forced vol present)");
    for pct in [10.0, 20.0, 30.0, 40.0, 50.0] {
        let thr = percentile(&imps, pct);
        sep_print(&decided, |s| s.flow.vol_dir > 0.0 && s.flow.impact_ratio <= thr, -1, "imp<=", thr, pct);
    }
    // 2. EXHAUSTION -> REVERSAL: LOW decel_ratio = push fading. Lower pctiles.
    println!("  [2] EXHAUSTION -> predicts REVERSAL  (gate: decel_ratio <= thr, early flow present)");
    for pct in [10.0, 20.0, 30.0, 40.0, 50.0] {
        let thr = percentile(&decs, pct);
        sep_print(&decided, |s| s.flow.early_dir_vol > 0.0 && s.flow.decel_ratio <= thr, -1, "dec<=", thr, pct);
    }
    // 3. EASY-PUSH -> CONTINUATION: HIGH impact = far on little vol. Upper pctiles.
    println!("  [3] EASY-PUSH -> predicts CONTINUATION  (gate: impact_ratio >= thr)");
    for pct in [50.0, 60.0, 70.0, 80.0, 90.0] {
        let thr = percentile(&imps, pct);
        sep_print(&decided, |s| s.flow.disp_bps > 0.0 && s.flow.impact_ratio >= thr, 1, "imp>=", thr, pct);
    }

    // 3-STATE GATED honest trades @ the configured impact-lo/impact-hi/flow-decay dials:
    // reversal gated on (ABSORBED|EXHAUSTED), continuation gated on (CONTINUATION).
    if cfg.state_gate {
        let mk_fee = cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS;
        println!(
            "  3-STATE GATED trades @ impact-lo {:.4}/impact-hi {:.4}/flow-decay {:.3} (net-of-fees, run-overs IN):",
            cfg.impact_lo, cfg.impact_hi, cfg.flow_decay
        );
        report_trades("  rev-MAKER (+ABS|EXH)", &caps_of(ds, |s| if s.rev_make_state_filled { s.rev_make_state } else { None }), mk_fee);
        report_trades("  rev-TAKER (+ABS|EXH)", &caps_of(ds, |s| if s.state_absorbed || s.state_exhausted { s.rev_take } else { None }), TAKER_RT_FEE_BPS);
        report_trades("  cont-TAKER (+CONT)", &caps_of(ds, |s| if s.state_continuation { s.cont_take } else { None }), TAKER_RT_FEE_BPS);
    }
    // gated honest trades (only the gates that are switched on).
    if cfg.absorb_gate || cfg.exhaust_gate || cfg.push_gate {
        let mk_fee = cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS;
        println!("  GATED honest trades @ configured thresholds (net-of-fees, run-overs IN):");
        if cfg.absorb_gate {
            report_trades("  rev-MAKER (+absorb-flow)", &caps_of(ds, |s| if s.rev_make_absorb_filled { s.rev_make_absorb } else { None }), mk_fee);
            report_trades("  rev-TAKER (+absorb-flow)", &caps_of(ds, |s| if s.absorb_confirm { s.rev_take } else { None }), TAKER_RT_FEE_BPS);
        }
        if cfg.exhaust_gate {
            report_trades("  rev-MAKER (+exhaust-flow)", &caps_of(ds, |s| if s.rev_make_exhaust_filled { s.rev_make_exhaust } else { None }), mk_fee);
            report_trades("  rev-TAKER (+exhaust-flow)", &caps_of(ds, |s| if s.exhaust_confirm { s.rev_take } else { None }), TAKER_RT_FEE_BPS);
        }
        if cfg.push_gate {
            report_trades("  cont-TAKER (+push-flow)", &caps_of(ds, |s| if s.push_confirm { s.cont_take } else { None }), TAKER_RT_FEE_BPS);
        }
    }
}

// ----------------------------------------------------------------------------
// main / CLI
// ----------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let mut cfg = Cfg {
        root: PathBuf::from("/root/chd/fresh/ticks"),
        coin: "ETH".to_string(),
        hours: (0..24).map(|h| format!("{h:02}")).collect(),
        cooldown_ns: 30 * 60_000_000_000,
        forward_ns: 90 * 60_000_000_000,
        sample_ns: 1_000_000_000,
        flow_window_ns: 60_000_000_000,
        top_n: 5,
        class_thr_bps: 20.0,
        rev_target_bps: 40.0,
        rev_stop_bps: 15.0,
        cont_target_bps: 60.0,
        cont_stop_bps: 20.0,
        hold_ns: 0,
        confirm_ns: 60_000_000_000,
        confirm_imb: 0.10,
        confirm_trend_max: 0.0,
        maker_fee_bps: 1.5,
        flow_on: false,
        absorb_gate: false,
        exhaust_gate: false,
        push_gate: false,
        absorb_min: 0.0,
        exhaust_decel: 1.0,
        push_min: 0.0,
        impact_lo: 0.0,
        impact_hi: 1e9,
        flow_decay: 0.0,
        absorb_minvol: 0.0,
        state_gate: false,
        min_oidrop: 0.0,
        dyn_stop: false,
        stop_buffer_bps: 5.0,
        rr_mult: 0.0,
        flow_exit: false,
        flow_exit_win_ns: 10_000_000_000,
        flow_exit_k: 2.0,
        maker_rev: false,
    };
    let mut dates: Vec<String> = Vec::new();
    let mut lookbacks: Vec<u64> = vec![30 * 60_000_000_000];
    let mut range_maxes: Vec<f64> = vec![50.0];
    let mut sweep_margins: Vec<f64> = vec![10.0];
    let mut hold: Option<u64> = None;
    let mut dump: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--root" => cfg.root = PathBuf::from(val()?),
            "--coin" => cfg.coin = val()?,
            "--dates" => dates = val()?.split(',').map(|s| s.trim().to_string()).collect(),
            "--hours" => {
                let h = val()?;
                cfg.hours = if h == "all" {
                    (0..24).map(|x| format!("{x:02}")).collect()
                } else {
                    h.split(',').map(|s| s.trim().to_string()).collect()
                };
            }
            "--lookbacks" => lookbacks = parse_dur_list(&val()?)?,
            "--range-max" => range_maxes = parse_f64_list(&val()?)?,
            "--sweep-margin" => sweep_margins = parse_f64_list(&val()?)?,
            "--cooldown" => cfg.cooldown_ns = parse_dur(&val()?)?,
            "--forward" => cfg.forward_ns = parse_dur(&val()?)?,
            "--sample" => cfg.sample_ns = parse_dur(&val()?)?,
            "--flow-window" => cfg.flow_window_ns = parse_dur(&val()?)?,
            "--top" => cfg.top_n = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--class-thr" => cfg.class_thr_bps = val()?.parse().map_err(|e| format!("class-thr: {e}"))?,
            "--rev-target" => cfg.rev_target_bps = val()?.parse().map_err(|e| format!("rev-target: {e}"))?,
            "--rev-stop" => cfg.rev_stop_bps = val()?.parse().map_err(|e| format!("rev-stop: {e}"))?,
            "--cont-target" => cfg.cont_target_bps = val()?.parse().map_err(|e| format!("cont-target: {e}"))?,
            "--cont-stop" => cfg.cont_stop_bps = val()?.parse().map_err(|e| format!("cont-stop: {e}"))?,
            "--hold" => hold = Some(parse_dur(&val()?)?),
            "--confirm-window" => cfg.confirm_ns = parse_dur(&val()?)?,
            "--confirm-imb" => cfg.confirm_imb = val()?.parse().map_err(|e| format!("confirm-imb: {e}"))?,
            "--confirm-trend-max" => cfg.confirm_trend_max = val()?.parse().map_err(|e| format!("confirm-trend-max: {e}"))?,
            "--maker-fee" => cfg.maker_fee_bps = val()?.parse().map_err(|e| format!("maker-fee: {e}"))?,
            "--flow" => cfg.flow_on = true,
            "--absorb-confirm" => { cfg.absorb_gate = true; cfg.flow_on = true; }
            "--absorb-min" => cfg.absorb_min = val()?.parse().map_err(|e| format!("absorb-min: {e}"))?,
            "--exhaust-confirm" => { cfg.exhaust_gate = true; cfg.flow_on = true; }
            "--exhaust-decel" => cfg.exhaust_decel = val()?.parse().map_err(|e| format!("exhaust-decel: {e}"))?,
            "--push-confirm" => { cfg.push_gate = true; cfg.flow_on = true; }
            "--push-min" => cfg.push_min = val()?.parse().map_err(|e| format!("push-min: {e}"))?,
            "--impact-lo" => { cfg.impact_lo = val()?.parse().map_err(|e| format!("impact-lo: {e}"))?; cfg.flow_on = true; }
            "--impact-hi" => { cfg.impact_hi = val()?.parse().map_err(|e| format!("impact-hi: {e}"))?; cfg.flow_on = true; }
            "--flow-decay" => { cfg.flow_decay = val()?.parse().map_err(|e| format!("flow-decay: {e}"))?; cfg.flow_on = true; }
            "--absorb-minvol" => cfg.absorb_minvol = val()?.parse().map_err(|e| format!("absorb-minvol: {e}"))?,
            "--state-gate" => { cfg.state_gate = true; cfg.flow_on = true; }
            "--min-oidrop" => { cfg.min_oidrop = val()?.parse().map_err(|e| format!("min-oidrop: {e}"))?; cfg.flow_on = true; }
            "--dyn-stop" => cfg.dyn_stop = true,
            "--stop-buffer" => cfg.stop_buffer_bps = val()?.parse().map_err(|e| format!("stop-buffer: {e}"))?,
            "--rr" => cfg.rr_mult = val()?.parse().map_err(|e| format!("rr: {e}"))?,
            "--flow-exit" => cfg.flow_exit = true,
            "--flow-exit-win" => cfg.flow_exit_win_ns = parse_dur(&val()?)?,
            "--flow-exit-k" => cfg.flow_exit_k = val()?.parse().map_err(|e| format!("flow-exit-k: {e}"))?,
            "--maker-rev" => cfg.maker_rev = true,
            "--dump" => dump = Some(val()?),
            other => return Err(format!("unknown arg {other}")),
        }
    }
    if dates.is_empty() {
        return Err("missing --dates (comma list)".into());
    }
    if cfg.top_n == 0 {
        return Err("--top must be >= 1".into());
    }
    cfg.hold_ns = hold.unwrap_or(cfg.forward_ns);

    println!("sweepscope - sideways stop-run / liquidity-sweep swing study (analysis only, no trading)");
    println!("coin {}  dates {}", cfg.coin, dates.join(","));
    println!(
        "cooldown {}  forward {}  hold {}  sample {}  flow-window {}  top {}",
        fmt_ns(cfg.cooldown_ns), fmt_ns(cfg.forward_ns), fmt_ns(cfg.hold_ns), fmt_ns(cfg.sample_ns), fmt_ns(cfg.flow_window_ns), cfg.top_n
    );
    println!(
        "classify: CONTINUATION if extends >= {:.0}bps past the sweep before re-entering the range, else REVERSAL on re-entry",
        cfg.class_thr_bps
    );
    println!(
        "REVERSAL trade: target {:.0}bps / stop {:.0}bps   CONTINUATION trade: target {:.0}bps / stop {:.0}bps",
        cfg.rev_target_bps, cfg.rev_stop_bps, cfg.cont_target_bps, cfg.cont_stop_bps
    );
    println!(
        "ORDERFLOW CONFIRM: absorption shift >= {:.2} (reversal) / follow-through return <= {:.2} (continuation), read at fire+{}",
        cfg.confirm_imb, cfg.confirm_trend_max, fmt_ns(cfg.confirm_ns)
    );
    if cfg.dyn_stop || cfg.flow_exit || cfg.maker_rev {
        println!(
            "DYNAMIC EXIT ON: dyn-stop {} (buffer {:.0}bps, rr {:.2})  flow-exit {} (win {}, k {:.2})  maker-rev {} (fee ~{:.1}bps RT)",
            cfg.dyn_stop, cfg.stop_buffer_bps, cfg.rr_mult, cfg.flow_exit, fmt_ns(cfg.flow_exit_win_ns), cfg.flow_exit_k,
            cfg.maker_rev, cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS
        );
    }
    let lb_lbl: Vec<String> = lookbacks.iter().map(|&l| fmt_ns(l)).collect();
    println!(
        "SWEEP grid: lookback L {:?}  x  range-max {:?}bps  x  sweep-margin {:?}bps",
        lb_lbl, range_maxes, sweep_margins
    );
    if cfg.flow_on {
        println!(
            "FLOW-vs-IMPACT separator ON (read <= entry): absorb-min {:.4} (rev)  exhaust-decel {:.4} (rev)  push-min {:.4} (cont)   gates[absorb={} exhaust={} push={}]",
            cfg.absorb_min, cfg.exhaust_decel, cfg.push_min, cfg.absorb_gate, cfg.exhaust_gate, cfg.push_gate
        );
        println!(
            "  3-STATE master variable = |microprice disp (bps)| / forced aggressive vol:  ABSORBED impact<={:.4}  CONTINUATION impact>={:.4}  EXHAUSTED decel<={:.3}  absorb-minvol {:.3}  state-gate {}",
            cfg.impact_lo, cfg.impact_hi, cfg.flow_decay, cfg.absorb_minvol, cfg.state_gate
        );
        if cfg.min_oidrop > 0.0 {
            println!("  OI-DROP GATE ON: only act on sweeps with oi_drop_pct >= {:.2}% (reported WITH and WITHOUT)", cfg.min_oidrop);
        }
    }

    // build the (L, range-max, sweep-margin) grid (knob-bite: the sweep set must move).
    let mut combos: Vec<(u64, f64, f64)> = Vec::new();
    for &l in &lookbacks {
        for &r in &range_maxes {
            for &s in &sweep_margins {
                combos.push((l, r, s));
            }
        }
    }
    let mut pooled: Vec<Vec<SweepStat>> = vec![Vec::new(); combos.len()];

    let mut loaded_days: Vec<String> = Vec::new();
    for day in &dates {
        let fc = FeedConfig {
            root: cfg.root.clone(),
            coin: cfg.coin.clone(),
            ref_symbols: Vec::new(),
            lead_symbols: Vec::new(),
            date: day.clone(),
            hours: cfg.hours.clone(),
            exec_latency_ns: 0,
            ref_latency_ns: 0,
        };
        let evs = match load_window(&fc) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  skip {day}: {e}");
                continue;
            }
        };
        if evs.is_empty() {
            eprintln!("  skip {day}: empty");
            continue;
        }
        eprintln!("  {day}: {} events", evs.len());
        for (ci, &(l, r, s)) in combos.iter().enumerate() {
            let st = scan_day(&cfg, day, &evs, l, r, s);
            pooled[ci].extend(st);
        }
        loaded_days.push(day.clone());
    }
    let num_days = loaded_days.len();

    for (ci, &(l, r, s)) in combos.iter().enumerate() {
        println!("\n==================== L={}  range-max={r}bps  sweep-margin={s}bps ====================", fmt_ns(l));
        for day in &loaded_days {
            let sub: Vec<&SweepStat> = pooled[ci].iter().filter(|x| &x.day == day).collect();
            report_day_line(day, &sub, &cfg);
        }
        let all: Vec<&SweepStat> = pooled[ci].iter().collect();
        report_pooled("POOLED (all days)", &all, &cfg, num_days);
    }

    if let Some(path) = &dump {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| format!("dump: {e}"))?;
        // flow columns are ADDITIVE: only present with --flow (default dump byte-preserved).
        let flow_hdr = if cfg.flow_on {
            ",vol_abs,vol_signed,vol_dir,disp_bps,max_ext_bps,absorption_ratio,impact_ratio,early_dir_vol,late_dir_vol,decel_ratio,absorb_confirm,exhaust_confirm,push_confirm,rev_make_absorb,rev_make_exhaust"
        } else {
            ""
        };
        writeln!(
            f,
            "day,l_ns,range_max,sweep_margin,fire_ts,dir_up,range_lo,range_hi,range_width_bps,mid,entry_px,oi_drop_pct,net_flow,class,cont_ext_bps,rev_back_bps,decision_ms,imb_start,imb_end,rev_confirm,cont_confirm,rev_take,rev_make,rev_make_filled,rev_make_gated,cont_take{flow_hdr}"
        ).map_err(|e| format!("dump: {e}"))?;
        for col in &pooled {
            for c in col {
                let flow_cols = if cfg.flow_on {
                    format!(
                        ",{:.4},{:.4},{:.4},{:.2},{:.2},{:.4},{:.6},{:.4},{:.4},{:.4},{},{},{},{},{}",
                        c.flow.vol_abs, c.flow.vol_signed, c.flow.vol_dir, c.flow.disp_bps, c.flow.max_ext_bps,
                        c.flow.absorption_ratio, c.flow.impact_ratio, c.flow.early_dir_vol, c.flow.late_dir_vol, c.flow.decel_ratio,
                        c.absorb_confirm, c.exhaust_confirm, c.push_confirm,
                        c.rev_make_absorb.map_or("NA".to_string(), |x| format!("{x:.3}")),
                        c.rev_make_exhaust.map_or("NA".to_string(), |x| format!("{x:.3}"))
                    )
                } else {
                    String::new()
                };
                writeln!(
                    f,
                    "{},{},{},{},{},{},{:.4},{:.4},{:.2},{:.4},{:.4},{:.4},{:.4},{},{:.2},{:.2},{},{:.4},{:.4},{},{},{},{},{},{},{}{flow_cols}",
                    c.day, c.l_ns, c.range_max, c.sweep_margin, c.fire_ts, c.dir_up,
                    c.range_lo, c.range_hi, c.range_width_bps, c.mid, c.entry_px,
                    c.oi_drop_pct, c.net_flow, c.class, c.cont_ext_bps, c.rev_back_bps,
                    c.decision_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.imb_start, c.imb_end, c.rev_confirm, c.cont_confirm,
                    c.rev_take.map_or("NA".to_string(), |x| format!("{x:.3}")),
                    c.rev_make.map_or("NA".to_string(), |x| format!("{x:.3}")),
                    c.rev_make_filled,
                    c.rev_make_gated.map_or("NA".to_string(), |x| format!("{x:.3}")),
                    c.cont_take.map_or("NA".to_string(), |x| format!("{x:.3}"))
                ).map_err(|e| format!("dump: {e}"))?;
            }
        }
        let total: usize = pooled.iter().map(Vec::len).sum();
        eprintln!("dumped {total} sweep rows to {path}");
    }

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
/// OPTION A - REVERSAL taker with a STRUCTURAL (dynamic) stop placed just BEYOND
/// the sweep WICK (the adverse extreme over [fire_ts, entry_ts]) plus a buffer,
/// instead of a fixed bps stop. `long` = our reversal side (true for a DOWN sweep
/// => buy; false for an UP sweep => short). The wick is the adverse extreme by
/// entry: MAX microprice for a short, MIN for a long. Stop distance =
/// |entry - wick| + `buffer_bps`. Target = `rr` * stop-distance when `rr` > 0
/// (dynamic R:R), else fixed `target_bps`. No-lookahead: the wick is read only
/// over [fire_ts, entry_ts] (<= entry). Returns (signed_bps, stop_dist_bps). Pure.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn first_touch_struct(
    path: &[(u64, f64)],
    fire_ts: u64,
    entry_ts: u64,
    long: bool,
    target_bps: f64,
    rr: f64,
    buffer_bps: f64,
    hold_ns: u64,
) -> Option<(f64, f64)> {
    let start = path.iter().position(|&(ts, _)| ts >= entry_ts)?;
    let entry_px = path[start].1;
    let entry_t = path[start].0;
    if entry_px <= 0.0 {
        return None;
    }
    // adverse wick over [fire, entry] (no-lookahead): MAX for a short, MIN for a long.
    let mut wick = entry_px;
    for &(ts, px) in path {
        if ts < fire_ts || ts > entry_ts {
            continue;
        }
        if long {
            if px < wick {
                wick = px;
            }
        } else if px > wick {
            wick = px;
        }
    }
    let raw = if long {
        (entry_px - wick) / entry_px * 10_000.0
    } else {
        (wick - entry_px) / entry_px * 10_000.0
    };
    let stop_dist = raw.max(0.0) + buffer_bps;
    let tgt = if rr > 0.0 { rr * stop_dist } else { target_bps };
    let s = if long { 1.0 } else { -1.0 };
    let mut last = 0.0f64;
    for &(ts, px) in &path[start..] {
        let signed = s * (px - entry_px) / entry_px * 10_000.0;
        last = signed;
        if tgt > 0.0 && signed >= tgt {
            return Some((signed, stop_dist));
        }
        if stop_dist > 0.0 && signed <= -stop_dist {
            return Some((signed, stop_dist));
        }
        if ts.saturating_sub(entry_t) >= hold_ns {
            return Some((signed, stop_dist));
        }
    }
    Some((last, stop_dist))
}

/// OPTION B - REVERSAL taker with NO price stop: exit when the FORCED (break-
/// direction) aggressive flow RE-ACCELERATES after entry (the manipulation turned
/// into a real trend), else take `target_bps` or `hold_ns`. `long` = our reversal
/// side; `break_buy` = the break-direction aggressor (UP sweep => true). The exit
/// fires when the break-dir aggressive-volume RATE over a trailing `flow_win_ns`
/// window reaches `reaccel_k` * `base_rate` (base_rate = the entry-window forced-
/// flow rate => the dial is self-calibrated per sweep, so units transfer across
/// assets). `trades` MUST be ascending in ts. Returns signed bps captured. Pure.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn first_touch_flow_exit(
    path: &[(u64, f64)],
    trades: &[(u64, f64, bool, f64)],
    entry_ts: u64,
    long: bool,
    break_buy: bool,
    target_bps: f64,
    hold_ns: u64,
    flow_win_ns: u64,
    base_rate: f64,
    reaccel_k: f64,
) -> Option<f64> {
    let start = path.iter().position(|&(ts, _)| ts >= entry_ts)?;
    let entry_px = path[start].1;
    let entry_t = path[start].0;
    if entry_px <= 0.0 {
        return None;
    }
    let s = if long { 1.0 } else { -1.0 };
    let eps = 1e-12;
    let thr_rate = reaccel_k * base_rate.max(eps);
    let win = flow_win_ns.max(1);
    let (mut hi, mut lo, mut vsum) = (0usize, 0usize, 0.0f64);
    let mut last = 0.0f64;
    for &(ts, px) in &path[start..] {
        let signed = s * (px - entry_px) / entry_px * 10_000.0;
        last = signed;
        if target_bps > 0.0 && signed >= target_bps {
            return Some(signed);
        }
        // trailing break-dir aggressive volume over [ts-win, ts] (ascending trades).
        let lo_bound = ts.saturating_sub(win);
        while hi < trades.len() && trades[hi].0 <= ts {
            let (_t, _p, buy, q) = trades[hi];
            if buy == break_buy && q.is_finite() && q > 0.0 {
                vsum += q;
            }
            hi += 1;
        }
        while lo < hi && trades[lo].0 < lo_bound {
            let (_t, _p, buy, q) = trades[lo];
            if buy == break_buy && q.is_finite() && q > 0.0 {
                vsum -= q;
            }
            lo += 1;
        }
        let rate = vsum / win as f64;
        if reaccel_k > 0.0 && base_rate > eps && ts > entry_t && rate >= thr_rate {
            return Some(signed);
        }
        if ts.saturating_sub(entry_t) >= hold_ns {
            return Some(signed);
        }
    }
    Some(last)
}

/// OPTION A (maker) - REVERSAL MAKER resting at the swept `edge`, exit by a
/// STRUCTURAL wick stop (beyond the adverse extreme over [fire, fill] + buffer),
/// target = `rr` * stop (or fixed `target_bps` when rr<=0). Fill rule + no-lookahead
/// hold-gate identical to `simulate_maker_reversal`. Fee escape ~6bps RT (maker in +
/// taker out) vs ~9bps taker. Returns (fill_ts, signed_bps, stop_dist_bps). Pure.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn simulate_maker_reversal_struct(
    path: &[(u64, f64)],
    trades: &[(u64, f64, bool, f64)],
    arm_ts: u64,
    fire_ts: u64,
    edge: f64,
    dir_up: bool,
    target_bps: f64,
    rr: f64,
    buffer_bps: f64,
    hold_ns: u64,
    hold_gate: Option<(u64, bool)>,
) -> Option<(u64, f64, f64)> {
    if edge <= 0.0 || path.is_empty() {
        return None;
    }
    let mut fill_ts: Option<u64> = None;
    for &(ts, px, buy, _q) in trades {
        if ts < arm_ts {
            continue;
        }
        let through = if dir_up { buy && px >= edge } else { !buy && px <= edge };
        if through {
            fill_ts = Some(ts);
            break;
        }
    }
    let fill_ts = fill_ts?;
    let mut force_exit_ts: Option<u64> = None;
    if let Some((ce, ok)) = hold_gate {
        if !ok {
            if fill_ts > ce {
                return None;
            }
            force_exit_ts = Some(ce);
        }
    }
    // structural stop: adverse wick over [fire, fill] (no-lookahead).
    let mut wick = edge;
    for &(ts, px) in path {
        if ts < fire_ts || ts > fill_ts {
            continue;
        }
        if dir_up {
            if px > wick {
                wick = px;
            }
        } else if px < wick {
            wick = px;
        }
    }
    let raw = if dir_up {
        (wick - edge) / edge * 10_000.0
    } else {
        (edge - wick) / edge * 10_000.0
    };
    let stop_dist = raw.max(0.0) + buffer_bps;
    let tgt = if rr > 0.0 { rr * stop_dist } else { target_bps };
    let s = if dir_up { -1.0 } else { 1.0 };
    let mut last = 0.0f64;
    let mut started = false;
    for &(ts, px) in path {
        if ts < fill_ts {
            continue;
        }
        started = true;
        let signed = s * (px - edge) / edge * 10_000.0;
        last = signed;
        if tgt > 0.0 && signed >= tgt {
            return Some((fill_ts, signed, stop_dist));
        }
        if stop_dist > 0.0 && signed <= -stop_dist {
            return Some((fill_ts, signed, stop_dist));
        }
        if let Some(fe) = force_exit_ts {
            if ts >= fe {
                return Some((fill_ts, signed, stop_dist));
            }
        }
        if ts.saturating_sub(fill_ts) >= hold_ns {
            return Some((fill_ts, signed, stop_dist));
        }
    }
    if started { Some((fill_ts, last, stop_dist)) } else { Some((fill_ts, 0.0, stop_dist)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_stop_target_win() {
        // UP sweep => reversal SHORT (long=false). Wick (max over [fire,entry]) =
        // 101.2; entry at t=1s px=101.0 => stop ~ (101.2-101.0) + buffer. Price snaps
        // down to 100 -> short from 101 ~+99bps -> target (40) hit.
        let path = vec![(0u64, 101.2), (1_000_000_000, 101.0), (2_000_000_000, 100.0)];
        let (cap, sd) = first_touch_struct(&path, 0, 1_000_000_000, false, 40.0, 0.0, 5.0, 30_000_000_000).unwrap();
        assert!(cap >= 40.0, "rode the snapback to target, got {cap}");
        assert!(sd > 5.0 && sd < 40.0, "structural stop ~ wick dist + buffer, got {sd}");
    }

    #[test]
    fn struct_stop_beyond_wick_caps_the_loss() {
        // UP sweep short, structural stop ~ wick(101.2)+buffer. Price keeps running
        // up THROUGH the stop -> exits near -stop_dist (not an unbounded loss).
        let path = vec![(0u64, 101.2), (1_000_000_000, 101.0), (1_500_000_000, 101.3), (2_000_000_000, 102.0)];
        let (cap, sd) = first_touch_struct(&path, 0, 1_000_000_000, false, 1000.0, 0.0, 5.0, 30_000_000_000).unwrap();
        assert!(cap <= -sd, "stopped at/through the structural stop, got cap {cap} sd {sd}");
        assert!(cap > -(sd + 15.0), "stop caps the loss near the level, got cap {cap} sd {sd}");
    }

    #[test]
    fn flow_exit_bails_when_break_flow_reaccelerates() {
        // DOWN sweep => reversal LONG, break_buy=false (break = aggressive sells).
        // A burst of aggressive SELLS after entry re-accelerates the break -> exit.
        let path = vec![(0u64, 100.0), (1_000_000_000, 99.99), (2_000_000_000, 99.5)];
        let trades = vec![(1_500_000_000u64, 99.9, false, 100.0)];
        let cap = first_touch_flow_exit(&path, &trades, 0, true, false, 1000.0, 30_000_000_000, 1_000_000_000, 1e-9, 1.0).unwrap();
        assert!(cap < 0.0, "bailed as the break flow re-accelerated against the long, got {cap}");
    }

    #[test]
    fn flow_exit_rides_to_target_without_reaccel() {
        // DOWN sweep reversal LONG. Only WITH-reversal buys print (not break-dir) ->
        // no early exit; price reverts up to the target.
        let path = vec![(0u64, 100.0), (1_000_000_000, 100.3), (2_000_000_000, 100.5)];
        let trades = vec![(1_500_000_000u64, 100.4, true, 100.0)];
        let cap = first_touch_flow_exit(&path, &trades, 0, true, false, 40.0, 30_000_000_000, 1_000_000_000, 0.001, 1.0).unwrap();
        assert!(cap >= 40.0, "rode the reversal to target with no break-flow re-accel, got {cap}");
    }

    #[test]
    fn maker_struct_rides_snapback_with_structural_stop() {
        // UP sweep: rest SELL at edge 101; forced BUY through at 101.2 (t=0.5s) fills.
        // wick over [fire,fill] ~ 101.2 -> stop beyond it; price snaps to 100 = win.
        let path = vec![(0u64, 101.2), (500_000_000, 101.0), (1_000_000_000, 100.5), (2_000_000_000, 100.0)];
        let trades = vec![(500_000_000u64, 101.2, true, 1.0)];
        let (fts, bps, sd) = simulate_maker_reversal_struct(&path, &trades, 0, 0, 101.0, true, 0.0, 2.0, 5.0, 30_000_000_000, None).unwrap();
        assert_eq!(fts, 500_000_000);
        assert!(bps > 0.0, "rode the snapback short, got {bps}");
        assert!(sd > 5.0, "structural stop includes the wick excursion, got {sd}");
    }

    #[test]
    fn parse_dur_units() {
        assert_eq!(parse_dur("30m").unwrap(), 30 * 60_000_000_000);
        assert_eq!(parse_dur("90m").unwrap(), 90 * 60_000_000_000);
        assert_eq!(parse_dur("60s").unwrap(), 60_000_000_000);
        assert_eq!(parse_dur("0").unwrap(), 0);
        assert!(parse_dur("-1s").is_err());
    }

    #[test]
    fn range_width_and_sideways() {
        // 100..101 around mid 100.5 = ~99.5bps wide.
        let w = range_width_bps(100.0, 101.0);
        assert!((w - 99.5).abs() < 1.0, "got {w}");
        // degenerate / inverted ranges are 0 width.
        assert!(range_width_bps(0.0, 0.0).abs() < 1e-12);
        assert!(range_width_bps(101.0, 100.0).abs() < 1e-12);
    }

    #[test]
    fn detect_sweep_both_sides() {
        // range [100, 101]; margin 10bps. hi*1.001 = 101.101, lo*0.999 = 99.9.
        assert_eq!(detect_sweep(101.2, 100.0, 101.0, 10.0), Some(true), "poke above hi+margin = UP");
        assert_eq!(detect_sweep(99.8, 100.0, 101.0, 10.0), Some(false), "poke below lo-margin = DOWN");
        // inside the range or not past the margin = no sweep.
        assert_eq!(detect_sweep(100.5, 100.0, 101.0, 10.0), None);
        assert_eq!(detect_sweep(101.05, 100.0, 101.0, 10.0), None, "past hi but inside the margin");
    }

    #[test]
    fn classify_continuation() {
        // UP sweep, entry just above hi=101 at 101.1; price RUNS to 102 (~+88bps)
        // before ever coming back below 101 => CONTINUATION.
        let entry = 101.1;
        let path = vec![(0u64, 101.1), (1_000_000_000, 101.5), (2_000_000_000, 102.0)];
        let oc = classify_outcome(&path, 0, entry, 101.0, true, 20.0);
        assert_eq!(oc.class, 1, "continuation");
        assert!(oc.cont_ext_bps > 50.0, "extension measured, got {}", oc.cont_ext_bps);
        assert!(oc.decision_ns.is_some());
    }

    #[test]
    fn classify_reversal() {
        // UP sweep, entry 101.1; price snaps back below the hi=101 edge to 100 =>
        // REVERSAL (re-entered the range before extending class_thr).
        let entry = 101.1;
        let path = vec![(0u64, 101.1), (1_000_000_000, 100.8), (2_000_000_000, 100.0)];
        let oc = classify_outcome(&path, 0, entry, 101.0, true, 20.0);
        assert_eq!(oc.class, -1, "reversal");
        assert!(oc.rev_back_bps > 50.0, "reversal magnitude measured, got {}", oc.rev_back_bps);
    }

    #[test]
    fn classify_neither_when_it_hovers() {
        // UP sweep that pokes a touch above hi then drifts but never extends
        // class_thr nor cleanly re-enters below the edge within the path.
        let entry = 101.1;
        let path = vec![(0u64, 101.1), (1_000_000_000, 101.12), (2_000_000_000, 101.15)];
        let oc = classify_outcome(&path, 0, entry, 101.0, true, 100.0);
        assert_eq!(oc.class, 0, "neither within the horizon");
    }

    #[test]
    fn first_touch_target_before_stop_is_a_win() {
        // LONG from 100. Price rises to 100.5 (~+50bps) first => target 40 hit.
        let path = vec![(0u64, 100.0), (1_000_000_000, 100.3), (2_000_000_000, 100.5)];
        let cap = first_touch(&path, 0, true, 40.0, 15.0, 30_000_000_000).unwrap();
        assert!(cap >= 40.0, "target first, got {cap}");
    }

    #[test]
    fn first_touch_stop_before_target_is_a_loss() {
        // SHORT from 100 (long=false). Price RISES to 100.2 (+20bps against a short
        // => -20bps) hitting the 15bps stop BEFORE any favourable move.
        let path = vec![(0u64, 100.0), (1_000_000_000, 100.2), (2_000_000_000, 99.0)];
        let cap = first_touch(&path, 0, false, 40.0, 15.0, 30_000_000_000).unwrap();
        assert!(cap <= -15.0, "stop touched first => loss, got {cap}");
        assert!(cap > -40.0, "stop caps the loss, got {cap}");
    }

    #[test]
    fn first_touch_none_when_path_ends_before_entry() {
        let path = vec![(0u64, 100.0), (100_000_000, 100.0)];
        assert!(first_touch(&path, 800_000_000, true, 40.0, 15.0, 30_000_000_000).is_none());
    }

    #[test]
    fn maker_reversal_fills_and_rides_snapback() {
        // UP sweep: rest a SELL at the edge hi=101. A forced aggressive BUY prints
        // THROUGH at 101.2 (t=0.5s) -> FILL at the limit 101 (short). Price then
        // snaps back to 100.0 -> short from 101 -> ~+99bps captured.
        let path = vec![(0u64, 101.2), (500_000_000, 101.0), (1_000_000_000, 100.5), (2_000_000_000, 100.0)];
        let trades = vec![(500_000_000u64, 101.2, true, 1.0)]; // aggressive BUY through the edge
        let (fts, bps) = simulate_maker_reversal(&path, &trades, 0, 101.0, true, 1000.0, 50.0, 30_000_000_000, None)
            .expect("forced buy fills the resting sell");
        assert_eq!(fts, 500_000_000);
        assert!(bps > 80.0, "rode the snapback short 101->100, got {bps}");
    }

    #[test]
    fn maker_reversal_trender_runs_over_is_a_loss() {
        // UP sweep, SELL fills at edge 101, but price KEEPS RUNNING UP (a real
        // breakout / continuation) -> the resting short bleeds = a REAL loss that
        // MUST be counted (no conditioning). 101 -> 103 = ~-198bps before stop.
        let path = vec![(0u64, 101.2), (500_000_000, 101.5), (1_000_000_000, 102.5), (2_000_000_000, 103.0)];
        let trades = vec![(500_000_000u64, 101.2, true, 1.0)];
        let (_, bps) = simulate_maker_reversal(&path, &trades, 0, 101.0, true, 1000.0, 1000.0, 30_000_000_000, None)
            .expect("fills then runs over");
        assert!(bps < -100.0, "trender run-over must be a real loss, got {bps}");
    }

    #[test]
    fn maker_reversal_no_fill_no_trade() {
        // The tape never trades through the resting sell at edge 101 (only a SELL
        // prints, wrong side) -> NO FILL -> None (no trade, no cost).
        let path = vec![(0u64, 101.2), (500_000_000, 100.9), (1_000_000_000, 100.5)];
        let trades = vec![(500_000_000u64, 101.2, false, 1.0)]; // aggressive SELL, wrong side
        assert!(simulate_maker_reversal(&path, &trades, 0, 101.0, true, 40.0, 15.0, 30_000_000_000, None).is_none());
    }

    #[test]
    fn maker_reversal_down_sweep_buys_at_low_edge() {
        // DOWN sweep: rest a BUY at edge lo=100. A forced aggressive SELL prints
        // THROUGH at 99.8 -> FILL at 100 (long). Price snaps back up to 101 -> long
        // from 100 -> ~+100bps.
        let path = vec![(0u64, 99.8), (500_000_000, 100.0), (1_000_000_000, 100.6), (2_000_000_000, 101.0)];
        let trades = vec![(500_000_000u64, 99.8, false, 1.0)]; // aggressive SELL through the low edge
        let (_, bps) = simulate_maker_reversal(&path, &trades, 0, 100.0, false, 1000.0, 50.0, 30_000_000_000, None)
            .expect("forced sell fills the resting buy");
        assert!(bps > 80.0, "long snapback 100->101, got {bps}");
    }

    #[test]
    fn maker_reversal_hold_gate_cancels_unconfirmed_late_fill() {
        // UP sweep. The fill would print at 1.0s, AFTER the 800ms confirm read. The
        // absorption confirm FAILS -> the still-resting sell is CANCELLED -> no
        // trade. With confirm OK the same late fill is kept.
        let path = vec![(0u64, 101.2), (800_000_000, 101.1), (1_000_000_000, 101.05), (2_000_000_000, 100.0)];
        let trades = vec![(1_000_000_000u64, 101.2, true, 1.0)];
        assert!(
            simulate_maker_reversal(&path, &trades, 0, 101.0, true, 40.0, 1000.0, 30_000_000_000, Some((800_000_000, false))).is_none(),
            "unconfirmed order cancelled before the late fill"
        );
        assert!(
            simulate_maker_reversal(&path, &trades, 0, 101.0, true, 40.0, 1000.0, 30_000_000_000, Some((800_000_000, true))).is_some(),
            "confirmed order stays resting and fills"
        );
    }

    #[test]
    fn confirm_absorption_and_followthrough_are_mirrors() {
        // DOWN sweep hit the BID. ABSORPTION = bid depth RETURNS => imbalance rises
        // toward bid (imb_end > imb_start). +0.30 shift passes a 0.10 thr.
        assert!(confirm_absorption(-0.20, 0.10, true, 0.10), "bid returned => absorption (reversal)");
        assert!(!confirm_absorption(-0.20, -0.40, true, 0.10), "bid kept thinning => no absorption");
        // UP sweep hit the ASK: absorption when ask depth returns => imb FALLS.
        assert!(confirm_absorption(0.20, -0.10, false, 0.10), "ask returned => absorption");
        // FOLLOW-THROUGH is the mirror: hit-side liquidity STAYS pulled.
        assert!(confirm_followthrough(-0.20, -0.40, true, 0.0), "bid stayed pulled => follow-through (continuation)");
        assert!(!confirm_followthrough(-0.20, 0.20, true, 0.0), "bid returned => not follow-through");
    }

    #[test]
    fn tstat_zero_and_positive() {
        assert!(tstat(&[1.0]).abs() < 1e-12, "n<2 => 0");
        assert!(tstat(&[5.0, 5.0, 5.0]).abs() < 1e-12, "zero variance => 0");
        assert!(tstat(&[2.0, 3.0, 2.5, 3.5, 2.0]) > 2.0, "clearly positive => t>2");
    }

    // ---- NEW flow-vs-impact separator tests ----

    #[test]
    fn flow_absorption_high_when_volume_eats_move() {
        // UP sweep: lots of aggressive BUY volume, price barely moves -> ABSORBED.
        let (fire, end, fire_px) = (0u64, 60_000_000_000u64, 100.0);
        let trades = vec![
            (1_000_000_000u64, 100.0, true, 50.0),
            (2_000_000_000u64, 100.0, true, 50.0),
            (40_000_000_000u64, 100.0, true, 50.0),
        ];
        let path = vec![(0u64, 100.0), (60_000_000_000u64, 100.001)]; // ~0.1bp move
        let m = compute_flow(&trades, &path, fire, end, fire_px, true);
        assert!((m.vol_dir - 150.0).abs() < 1e-9, "forced buy vol, got {}", m.vol_dir);
        assert!(m.disp_bps < 1.0, "tiny displacement, got {}", m.disp_bps);
        assert!(m.absorption_ratio > 100.0, "high absorption, got {}", m.absorption_ratio);
        assert!(confirm_absorb_flow(&m, 50.0), "absorption confirmed");
        assert!(!confirm_push_flow(&m, 1.0), "NOT an easy push");
    }

    #[test]
    fn flow_easy_push_high_impact_low_volume() {
        // UP sweep: tiny volume, big price move -> EASY PUSH (vacuum).
        let (fire, end, fire_px) = (0u64, 60_000_000_000u64, 100.0);
        let trades = vec![(1_000_000_000u64, 100.0, true, 0.5)];
        let path = vec![(0u64, 100.0), (60_000_000_000u64, 101.0)]; // +100bps
        let m = compute_flow(&trades, &path, fire, end, fire_px, true);
        assert!((m.disp_bps - 100.0).abs() < 1.0, "big move, got {}", m.disp_bps);
        assert!(m.impact_ratio > 100.0, "high impact, got {}", m.impact_ratio);
        assert!(confirm_push_flow(&m, 50.0), "push confirmed");
        assert!(!confirm_absorb_flow(&m, 50.0), "low absorption");
    }

    #[test]
    fn flow_exhaustion_when_late_volume_fades() {
        // early half heavy, late half light -> decel small -> EXHAUSTED.
        let (fire, end, fire_px) = (0u64, 60_000_000_000u64, 100.0);
        let trades = vec![
            (5_000_000_000u64, 100.0, true, 80.0),   // early (<= 30s mid)
            (10_000_000_000u64, 100.0, true, 80.0),  // early
            (50_000_000_000u64, 100.0, true, 10.0),  // late
        ];
        let path = vec![(0u64, 100.0), (60_000_000_000u64, 100.1)];
        let m = compute_flow(&trades, &path, fire, end, fire_px, true);
        assert!((m.early_dir_vol - 160.0).abs() < 1e-9, "early, got {}", m.early_dir_vol);
        assert!((m.late_dir_vol - 10.0).abs() < 1e-9, "late, got {}", m.late_dir_vol);
        assert!(m.decel_ratio < 0.1, "late faded, got {}", m.decel_ratio); // 10/160
        assert!(confirm_exhaust_flow(&m, 0.5), "exhausted at decel 0.5");
        assert!(!confirm_exhaust_flow(&m, 0.05), "NOT exhausted at strict 0.05");
    }

    #[test]
    fn flow_window_is_no_lookahead() {
        // trades before fire and after window-end are EXCLUDED (read <= entry).
        let (fire, end, fire_px) = (10_000_000_000u64, 20_000_000_000u64, 100.0);
        let trades = vec![
            (5_000_000_000u64, 100.0, true, 99.0),   // before window -> excluded
            (15_000_000_000u64, 100.0, true, 7.0),   // in window
            (25_000_000_000u64, 100.0, true, 99.0),  // after window-end -> excluded
        ];
        let path = vec![(10_000_000_000u64, 100.0), (20_000_000_000u64, 100.0)];
        let m = compute_flow(&trades, &path, fire, end, fire_px, true);
        assert!((m.vol_dir - 7.0).abs() < 1e-9, "only in-window flow counts, got {}", m.vol_dir);
    }

    #[test]
    fn flow_dir_volume_follows_sweep_side() {
        // DOWN sweep: forced flow = aggressive SELLS; buys are against.
        let (fire, end, fire_px) = (0u64, 60_000_000_000u64, 100.0);
        let trades = vec![
            (1_000_000_000u64, 100.0, false, 30.0), // sell = forced (down sweep)
            (2_000_000_000u64, 100.0, true, 10.0),  // buy = against
        ];
        let path = vec![(0u64, 100.0), (60_000_000_000u64, 99.95)];
        let m = compute_flow(&trades, &path, fire, end, fire_px, false);
        assert!((m.vol_dir - 30.0).abs() < 1e-9, "down forced = sells, got {}", m.vol_dir);
        assert!((m.vol_abs - 40.0).abs() < 1e-9, "abs vol, got {}", m.vol_abs);
        assert!((m.vol_signed - (-20.0)).abs() < 1e-9, "30 sell - 10 buy, got {}", m.vol_signed);
    }

    #[test]
    fn separation_detects_lift_and_random() {
        // signal true mostly on reversals (class -1) -> P(rev|sig) > P(rev|!sig).
        let lift = vec![(true, -1i8), (true, -1), (true, -1), (true, 1), (false, 1), (false, 1), (false, -1), (false, 1)];
        let (pw, nw, pn, nn) = separation(&lift, -1);
        assert_eq!((nw, nn), (4, 4));
        assert!((pw - 75.0).abs() < 1e-9, "P(rev|sig), got {pw}");
        assert!((pn - 25.0).abs() < 1e-9, "P(rev|!sig), got {pn}");
        assert!(pw > pn, "signal lifts reversal prob");
        // random signal -> no lift (like the dead depth-imbalance confirm).
        let rnd = vec![(true, -1i8), (true, 1), (false, -1), (false, 1)];
        let (rpw, _, rpn, _) = separation(&rnd, -1);
        assert!((rpw - rpn).abs() < 1e-9, "random separator => zero lift");
    }

    #[test]
    fn percentile_basic() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((percentile(&v, 0.0) - 1.0).abs() < 1e-9);
        assert!((percentile(&v, 50.0) - 3.0).abs() < 1e-9);
        assert!((percentile(&v, 100.0) - 5.0).abs() < 1e-9);
        assert!((percentile(&[], 50.0)).abs() < 1e-12, "empty => 0");
    }

    // ---- NEW 3-state (master-variable) assignment tests ----

    /// Build a FlowMetrics with the master-variable fields set (avoids the
    /// field-reassign-with-default clippy lint by using a full struct literal).
    fn fm(disp: f64, vdir: f64, early: f64, late: f64) -> FlowMetrics {
        FlowMetrics {
            vol_abs: vdir,
            vol_signed: vdir,
            vol_dir: vdir,
            disp_bps: disp,
            max_ext_bps: disp,
            absorption_ratio: vdir / disp.max(1e-9),
            impact_ratio: disp / vdir.max(1e-9),
            early_dir_vol: early,
            late_dir_vol: late,
            decel_ratio: late / early.max(1e-9),
        }
    }

    #[test]
    fn assign_state_three_way() {
        // ABSORBED: lots of forced vol, tiny move -> LOW impact-per-vol (0.005).
        let a = fm(0.5, 100.0, 50.0, 50.0);
        assert_eq!(assign_flow_state(&a, 0.05, 0.5, 0.5, 1.0), FlowState::Absorbed);
        // CONTINUATION: big move on little vol -> HIGH impact-per-vol (10).
        let c = fm(50.0, 5.0, 2.5, 2.5);
        assert_eq!(assign_flow_state(&c, 0.05, 0.5, 0.5, 1.0), FlowState::Continuation);
        // EXHAUSTED: mid impact (0.1, between lo/hi), late flow faded hard (decel 0.11).
        let e = fm(5.0, 50.0, 45.0, 5.0);
        assert_eq!(assign_flow_state(&e, 0.05, 0.5, 0.5, 1.0), FlowState::Exhausted);
        // UNCLASSIFIED: mid impact, flow steady (decel 1.0).
        let u = fm(5.0, 50.0, 25.0, 25.0);
        assert_eq!(assign_flow_state(&u, 0.05, 0.5, 0.5, 1.0), FlowState::Unclassified);
    }

    #[test]
    fn assign_state_absorb_needs_real_volume() {
        // LOW impact but NO forced volume -> not a real absorption (vol must clear minvol).
        let m = fm(0.0, 0.0, 0.0, 0.0);
        assert_eq!(assign_flow_state(&m, 0.05, 0.5, 0.5, 1.0), FlowState::Unclassified, "no forced vol => not absorbed");
        // forced vol present but BELOW absorb-minvol -> still not absorbed.
        let m2 = fm(0.2, 0.5, 0.25, 0.25); // impact 0.4 > lo, also vol 0.5 < minvol 1.0
        assert_eq!(assign_flow_state(&m2, 0.05, 0.5, 0.5, 1.0), FlowState::Unclassified);
    }

    #[test]
    fn flow_decay_knob_bites_state() {
        // mid impact (0.1, between lo/hi) so ONLY the decel dial decides EXHAUSTED.
        let m = fm(5.0, 50.0, 100.0, 30.0); // decel 0.30
        assert_eq!(assign_flow_state(&m, 0.05, 0.5, 0.20, 0.0), FlowState::Unclassified, "strict decay 0.20: 0.30 not exhausted");
        assert_eq!(assign_flow_state(&m, 0.05, 0.5, 0.50, 0.0), FlowState::Exhausted, "loose decay 0.50: exhausted");
    }

    #[test]
    fn impact_cut_knob_bites_state() {
        // a sweep with impact_ratio 0.1: low-cut at 0.05 -> NOT absorbed; at 0.20 -> absorbed.
        let m = fm(5.0, 50.0, 25.0, 25.0); // impact 0.1, decel 1.0 (no exhaustion)
        assert_eq!(assign_flow_state(&m, 0.05, 0.5, 0.5, 1.0), FlowState::Unclassified, "lo 0.05: 0.1 not <= 0.05");
        assert_eq!(assign_flow_state(&m, 0.20, 0.5, 0.5, 1.0), FlowState::Absorbed, "lo 0.20: 0.1 <= 0.20 -> absorbed");
        // raise the hi cut below 0.1 -> the same sweep flips to CONTINUATION.
        assert_eq!(assign_flow_state(&m, 0.05, 0.08, 0.5, 1.0), FlowState::Continuation, "hi 0.08: 0.1 >= 0.08 -> continuation");
    }
}
