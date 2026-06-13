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
    trades: &[(u64, f64, bool)],
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
    for &(ts, px, buy) in trades {
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
    };
    let mut cur_path: Vec<(u64, f64)> = Vec::new();
    let mut cur_trades: Vec<(u64, f64, bool)> = Vec::new();
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
                        cur_trades.push((now, ev.price.to_f64(), aggr == Side::Bid));
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
                }
                if now >= cur.end_ts {
                    // ensure the confirm read happened (short horizons guard).
                    if cur.imb_end.is_none() {
                        let (bd, ad) = top_depth(&book, cfg.top_n);
                        cur.imb_end = Some(imbalance(bd, ad));
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
fn finalize(cur: &Pending, path: &[(u64, f64)], trades: &[(u64, f64, bool)], cfg: &Cfg, day: &str, l_ns: u64, range_max: f64, sweep_margin: f64) -> SweepStat {
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
    let lb_lbl: Vec<String> = lookbacks.iter().map(|&l| fmt_ns(l)).collect();
    println!(
        "SWEEP grid: lookback L {:?}  x  range-max {:?}bps  x  sweep-margin {:?}bps",
        lb_lbl, range_maxes, sweep_margins
    );

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
        writeln!(
            f,
            "day,l_ns,range_max,sweep_margin,fire_ts,dir_up,range_lo,range_hi,range_width_bps,mid,entry_px,oi_drop_pct,net_flow,class,cont_ext_bps,rev_back_bps,decision_ms,imb_start,imb_end,rev_confirm,cont_confirm,rev_take,rev_make,rev_make_filled,rev_make_gated,cont_take"
        ).map_err(|e| format!("dump: {e}"))?;
        for col in &pooled {
            for c in col {
                writeln!(
                    f,
                    "{},{},{},{},{},{},{:.4},{:.4},{:.2},{:.4},{:.4},{:.4},{:.4},{},{:.2},{:.2},{},{:.4},{:.4},{},{},{},{},{},{},{}",
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
#[cfg(test)]
mod tests {
    use super::*;

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
        let trades = vec![(500_000_000u64, 101.2, true)]; // aggressive BUY through the edge
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
        let trades = vec![(500_000_000u64, 101.2, true)];
        let (_, bps) = simulate_maker_reversal(&path, &trades, 0, 101.0, true, 1000.0, 1000.0, 30_000_000_000, None)
            .expect("fills then runs over");
        assert!(bps < -100.0, "trender run-over must be a real loss, got {bps}");
    }

    #[test]
    fn maker_reversal_no_fill_no_trade() {
        // The tape never trades through the resting sell at edge 101 (only a SELL
        // prints, wrong side) -> NO FILL -> None (no trade, no cost).
        let path = vec![(0u64, 101.2), (500_000_000, 100.9), (1_000_000_000, 100.5)];
        let trades = vec![(500_000_000u64, 101.2, false)]; // aggressive SELL, wrong side
        assert!(simulate_maker_reversal(&path, &trades, 0, 101.0, true, 40.0, 15.0, 30_000_000_000, None).is_none());
    }

    #[test]
    fn maker_reversal_down_sweep_buys_at_low_edge() {
        // DOWN sweep: rest a BUY at edge lo=100. A forced aggressive SELL prints
        // THROUGH at 99.8 -> FILL at 100 (long). Price snaps back up to 101 -> long
        // from 100 -> ~+100bps.
        let path = vec![(0u64, 99.8), (500_000_000, 100.0), (1_000_000_000, 100.6), (2_000_000_000, 101.0)];
        let trades = vec![(500_000_000u64, 99.8, false)]; // aggressive SELL through the low edge
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
        let trades = vec![(1_000_000_000u64, 101.2, true)];
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
}
