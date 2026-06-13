//! `oiscope` - a RESEARCH / ANALYSIS tool (NOT a strategy) that DETECTS and
//! CHARACTERISES forced-flow LIQUIDATION CASCADES on Hyperliquid, reconstructed
//! from open-interest (OI) drops, and answers ONE honest question: is the
//! post-cascade price reversion real, big enough, and SLOW enough to capture at
//! our ~0.8-2.4s execution latency? (Lagshot died because the basis reversion
//! out-ran us; a cascade is a move that ALREADY printed, so we REACT to it
//! instead of racing a micro-reversion - latency should bind far less.)
//!
//! THESIS under test: a sharp OI DROP (positions force-closed) + a simultaneous
//! microprice SPIKE + net one-sided aggressive HL trade flow in the move
//! direction = a liquidation cascade. Forced SELLING (longs liquidated) drops
//! price + drops OI -> cascade DOWN; forced BUYING (shorts liquidated) ->
//! cascade UP. We detect both and record direction.
//!
//! It replays a real HL day (reusing `load_window` + `forge_book::OrderBook` -
//! sacred core untouched), DETECTS cascades on a rolling window W (OI-drop >= D%
//! AND |microprice move| >= P bps AND net one-sided flow in the move dir),
//! de-dups via a cooldown, then over a forward horizon CHARACTERISES the spike,
//! the overshoot/reversion (magnitude + time-to-half / time-to-full revert), and
//! the key tradeability measure: it simulates a SIMPLE reactive FADE entered at
//! a configurable DELAY (0 / 800 / 2000ms = our latency band), reporting the bps
//! captured at each delay with a t-stat across cascades. Deterministic and
//! no-lookahead (only `<= now` state is read). PRICE = HL top-N microprice (what
//! we would actually trade against), not mark.
//!
//! ```text
//! oiscope --root /root/chd/fresh/ticks --coin ETH
//!   --dates 2025-11-04,2025-11-20 --hours all
//!   --window 5s --oi-drop 0.2,0.5,1.0 --price-move 5,10,20
//!   --min-flow 0 --cooldown 30s --forward 60s --delays 0,800ms,2000ms
//!   --fade-revert 10 --fade-hold 30s --revert-frac 0.5 --top 5 --dump rows.csv
//! ```

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitCode;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, Side, UnixNanos};
use forgelag::{load_window, FeedConfig, LagEvent, LagKind, Role};

// ----------------------------------------------------------------------------
// small parse / math helpers (mirroring gapscope conventions)
// ----------------------------------------------------------------------------

/// Parse a duration like `5s`, `800ms`, `30s`, `250us`, `100ns` into ns.
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

/// Size-weighted top-N microprice, copied from gapscope / `FairValueOracle::micro`.
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

/// One-sample t-stat of a series against zero: mean / (sample-sd / sqrt(n)).
/// 0.0 when fewer than two points or zero variance (honest: no significance).
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
// ----------------------------------------------------------------------------
// PURE, TESTED measurement: reversion + reactive-fade-at-delay
// ----------------------------------------------------------------------------

/// Outcome of measuring the forward price path of one cascade.
#[derive(Clone, Copy, Debug, Default)]
struct Reversion {
    /// Peak excursion from the pre-cascade baseline IN the cascade direction (bps).
    /// This is the full SPIKE size (how far price ran during the cascade).
    spike_bps: f64,
    /// Maximum recovery back from the peak toward (and possibly through) the
    /// baseline over the horizon (bps). The headline REVERSION magnitude.
    revert_bps: f64,
    /// `revert_bps / spike_bps` (0.0 if no spike). >= 1.0 means it returned fully.
    revert_frac: f64,
    /// True if price returned all the way to the pre-cascade baseline (excursion <= 0).
    full_revert: bool,
    /// ns from fire to the first time half the spike was given back (None if never).
    tth_ns: Option<u64>,
    /// ns from fire to the first full revert to baseline (None if never).
    ttf_ns: Option<u64>,
    /// ns from fire to the peak of the spike.
    peak_ns: u64,
}

/// Measure the spike + reversion of a forward microprice `path` (each `(ts, px)`,
/// ts ascending, starting at the cascade fire). `baseline` = pre-cascade
/// microprice (the level a reversion would return to); `dir_down` = the cascade
/// moved price DOWN. Pure: depends only on the path.
#[must_use]
fn measure_reversion(path: &[(u64, f64)], fire_ts: u64, baseline: f64, dir_down: bool) -> Reversion {
    if path.is_empty() || baseline <= 0.0 {
        return Reversion::default();
    }
    let s = if dir_down { -1.0 } else { 1.0 };
    let exc = |px: f64| s * (px - baseline) / baseline * 10_000.0;
    // peak excursion in cascade direction.
    let mut peak = 0.0f64;
    let mut peak_idx = 0usize;
    let mut peak_ts = fire_ts;
    for (i, &(ts, px)) in path.iter().enumerate() {
        let e = exc(px);
        if e > peak {
            peak = e;
            peak_idx = i;
            peak_ts = ts;
        }
    }
    let spike_bps = peak.max(0.0);
    // recovery measured from the peak onward.
    let mut max_recovery = 0.0f64;
    let mut tth = None;
    let mut ttf = None;
    for &(ts, px) in &path[peak_idx..] {
        let e = exc(px);
        let rec = peak - e;
        if rec > max_recovery {
            max_recovery = rec;
        }
        if tth.is_none() && spike_bps > 0.0 && e <= 0.5 * spike_bps {
            tth = Some(ts.saturating_sub(fire_ts));
        }
        if ttf.is_none() && e <= 0.0 {
            ttf = Some(ts.saturating_sub(fire_ts));
        }
    }
    let revert_bps = max_recovery.max(0.0);
    let revert_frac = if spike_bps > 0.0 { revert_bps / spike_bps } else { 0.0 };
    Reversion {
        spike_bps,
        revert_bps,
        revert_frac,
        full_revert: ttf.is_some(),
        tth_ns: tth,
        ttf_ns: ttf,
        peak_ns: peak_ts.saturating_sub(fire_ts),
    }
}

/// Simulate a SIMPLE reactive FADE of the cascade, entered at `delay_ns` AFTER
/// the cascade fired (our execution latency), held until it gives us
/// `revert_target_bps` of favourable reversion OR `max_hold_ns` elapses (then
/// marked to market). Returns the bps CAPTURED (signed: negative if it trended
/// against us), or None if the path ended before the entry delay. Pure.
///
/// Down cascade -> we BUY (expect a bounce up); up cascade -> we SELL.
#[must_use]
fn simulate_fade(
    path: &[(u64, f64)],
    fire_ts: u64,
    dir_down: bool,
    delay_ns: u64,
    revert_target_bps: f64,
    max_hold_ns: u64,
) -> Option<f64> {
    let fade_sign = if dir_down { 1.0 } else { -1.0 };
    let entry_t = fire_ts.saturating_add(delay_ns);
    let mut start = None;
    for (i, &(ts, _)) in path.iter().enumerate() {
        if ts >= entry_t {
            start = Some(i);
            break;
        }
    }
    let start = start?;
    let entry_px = path[start].1;
    let entry_ts = path[start].0;
    if entry_px <= 0.0 {
        return None;
    }
    let mut last_fav = 0.0f64;
    for &(ts, px) in &path[start..] {
        let fav = fade_sign * (px - entry_px) / entry_px * 10_000.0;
        last_fav = fav;
        if fav >= revert_target_bps {
            return Some(fav);
        }
        if ts.saturating_sub(entry_ts) >= max_hold_ns {
            return Some(fav);
        }
    }
    Some(last_fav)
}

/// Simulate a TAKER MOMENTUM trade that goes WITH the cascade continuation: a DOWN
/// cascade -> SELL/short to ride the continued drop; an UP cascade -> BUY/long to ride
/// the continued rise. Entered at `delay_ns` AFTER the fire (our latency band). Exit by
/// FIRST-TOUCH over the forward path in strict time order, using ONLY forward data (no
/// lookahead): whichever of the PROFIT TARGET (`target_bps`, favourable in the cascade
/// direction = the overshoot) or the STOP (`stop_bps`, against us = a reverter) is hit
/// FIRST decides the trade; if neither is hit within `max_hold_ns`, mark to market.
/// Returns the signed bps captured (negative = the cascade reverted against us and we
/// stopped / bled). None if the path ended before the entry delay. EVERY entry's real
/// signed bps is returned - reverters that stop out count their loss (no conditioning).
/// Pure: depends only on the path.
#[must_use]
fn simulate_momentum(
    path: &[(u64, f64)],
    fire_ts: u64,
    dir_down: bool,
    delay_ns: u64,
    target_bps: f64,
    stop_bps: f64,
    max_hold_ns: u64,
) -> Option<f64> {
    // trade WITH the move: profit when price CONTINUES in the cascade direction.
    let mom_sign = if dir_down { -1.0 } else { 1.0 };
    let entry_t = fire_ts.saturating_add(delay_ns);
    let mut start = None;
    for (i, &(ts, _)) in path.iter().enumerate() {
        if ts >= entry_t {
            start = Some(i);
            break;
        }
    }
    let start = start?;
    let entry_px = path[start].1;
    let entry_ts = path[start].0;
    if entry_px <= 0.0 {
        return None;
    }
    let mut last = 0.0f64;
    for &(ts, px) in &path[start..] {
        let signed = mom_sign * (px - entry_px) / entry_px * 10_000.0;
        last = signed;
        // FIRST-TOUCH: whichever bound is breached first in time decides the trade.
        if target_bps > 0.0 && signed >= target_bps {
            return Some(signed);
        }
        if stop_bps > 0.0 && signed <= -stop_bps {
            return Some(signed);
        }
        if ts.saturating_sub(entry_ts) >= max_hold_ns {
            return Some(signed);
        }
    }
    Some(last)
}

/// Find the EXHAUSTION point in a cascade's forward path: the FIRST sample where
/// BOTH conditions hold, meaning the forced flow has RUN OUT so a snap-back is
/// likely:
///   (a) OI STOPS BLEEDING - the OI-drop over the trailing `stall_ns` window has
///       decelerated to <= `decel_frac` x the PEAK trailing OI-drop seen since the
///       fire (forced deleveraging fading). If OI never bled forward at all, this
///       leg is treated as satisfied (the flow was already spent at the fire).
///   (b) PRICE STALLS - price has not made a NEW extreme in the cascade direction
///       within the trailing `stall_ns` window (it stops extending).
/// `price` and `oi` are ALIGNED sample paths `(ts, value)`, ascending, both
/// starting at the fire (same indices/timestamps). At least one full `stall_ns`
/// must have elapsed since the fire before judging (so a window + a peak exist).
/// Returns the timestamp of the first qualifying sample, or None if exhaustion
/// never occurs within the path (=> that cascade is SKIPPED, no trade). Pure.
#[must_use]
fn find_exhaustion(
    price: &[(u64, f64)],
    oi: &[(u64, f64)],
    fire_ts: u64,
    dir_down: bool,
    stall_ns: u64,
    decel_frac: f64,
) -> Option<u64> {
    if price.len() < 2 || oi.len() != price.len() || stall_ns == 0 {
        return None;
    }
    // OI value at-or-before a target ts on the sampled path (None if before start).
    let oi_at = |target: u64| -> Option<f64> {
        let mut v = None;
        for &(ts, o) in oi {
            if ts <= target {
                v = Some(o);
            } else {
                break;
            }
        }
        v
    };
    // trailing OI-drop% over [t-stall, t] (positive = still bleeding). None if the
    // window reaches before the fire or the start OI is non-positive.
    let trailing_drop = |t: u64| -> Option<f64> {
        let lo = t.checked_sub(stall_ns)?;
        if lo < fire_ts {
            return None;
        }
        let o_start = oi_at(lo)?;
        let o_end = oi_at(t)?;
        if o_start <= 0.0 {
            return None;
        }
        Some((o_start - o_end) / o_start * 100.0)
    };
    let s = if dir_down { -1.0 } else { 1.0 };
    let mut extreme = price[0].1;
    let mut extreme_ts = price[0].0;
    let mut peak_rate = 0.0f64;
    for &(ts, px) in price {
        // (b) update the running cascade-direction extreme FIRST.
        if s * px > s * extreme {
            extreme = px;
            extreme_ts = ts;
        }
        // need a full trailing window since the fire before judging.
        if ts.saturating_sub(fire_ts) < stall_ns {
            continue;
        }
        let cur_rate = match trailing_drop(ts) {
            Some(r) => r,
            None => continue,
        };
        if cur_rate > peak_rate {
            peak_rate = cur_rate;
        }
        // (a) OI bleed has decelerated below the fraction of its peak rate.
        let oi_exhausted = if peak_rate > 0.0 {
            cur_rate <= decel_frac * peak_rate
        } else {
            true // OI never bled forward => forced flow already spent.
        };
        // (b) price has not extended (no new extreme) for the whole stall window.
        let price_stalled = ts.saturating_sub(extreme_ts) >= stall_ns;
        if oi_exhausted && price_stalled {
            return Some(ts);
        }
    }
    None
}

/// Decide whether a cascade PASSES the magnitude filter (Tweak 2). Uses ONLY
/// information realized AT or before the entry decision: `gate_spike_bps` = the
/// |detection-window price move| (window-start -> fire, already printed) and the
/// realized `oi_drop_pct`. It NEVER reads the forward peak (`Reversion::spike_bps`),
/// which would be lookahead. A threshold <= 0 disables that leg (baseline = both
/// off => always passes => byte-preserved). Pure.
#[must_use]
fn passes_magnitude(gate_spike_bps: f64, oi_drop_pct: f64, min_spike_bps: f64, min_oidrop_pct: f64) -> bool {
    (min_spike_bps <= 0.0 || gate_spike_bps >= min_spike_bps)
        && (min_oidrop_pct <= 0.0 || oi_drop_pct >= min_oidrop_pct)
}

/// Top-N depth imbalance in [-1, 1]: `(bid - ask) / (bid + ask)`. Positive = more
/// bid depth (book leans up). Copied from gapscope so the two tools agree exactly.
#[must_use]
fn imbalance(bid_depth: f64, ask_depth: f64) -> f64 {
    let t = bid_depth + ask_depth;
    if t <= 0.0 { 0.0 } else { (bid_depth - ask_depth) / t }
}

/// Top-N (bid_depth, ask_depth) in base units. Copied from gapscope.
#[must_use]
fn top_depth(book: &OrderBook, top_n: usize) -> (f64, f64) {
    let bid: f64 = book.bids_iter().take(top_n).map(|(_, q)| q.to_f64()).sum();
    let ask: f64 = book.asks_iter().take(top_n).map(|(_, q)| q.to_f64()).sum();
    (bid, ask)
}

/// Tweak 3 Part A confirm gate. A DOWN cascade was HIT on the BID (forced sells
/// consumed bids); LIQUIDITY RETURNING on the hit side = bid depth re-filling =>
/// top-N imbalance RISES (toward bid). An UP cascade was hit on the ASK => a
/// revert is supported when imbalance FALLS (toward ask). Returns true when the
/// toward-hit-side imbalance shift over the confirm window >= `min_shift`. Pure.
#[must_use]
fn confirm_revert(imb_start: f64, imb_end: f64, dir_down: bool, min_shift: f64) -> bool {
    let toward_hit = if dir_down { imb_end - imb_start } else { imb_start - imb_end };
    toward_hit >= min_shift
}

/// MOMENTUM trend-confirm (Tweak 5) - the MIRROR of `confirm_revert`. A DOWN cascade
/// was HIT on the BID; we trade WITH the continuation (short) only when LIQUIDITY
/// STAYS PULLED on the hit side, i.e. the top-N imbalance does NOT shift back toward
/// the hit side (the bid does NOT re-fill). An UP cascade was hit on the ASK; the
/// trend holds when ask depth stays gone. Returns true when the toward-hit-side shift
/// over the confirm window is <= `max_return` (default 0 = no meaningful return). The
/// fade wants liquidity to RETURN; momentum wants it to STAY GONE. Pure.
#[must_use]
fn confirm_trend(imb_start: f64, imb_end: f64, dir_down: bool, max_return: f64) -> bool {
    let toward_hit = if dir_down { imb_end - imb_start } else { imb_start - imb_end };
    toward_hit <= max_return
}

/// Tweak 3 Part B maker-fill feasibility. During the reversion leg (AT/after the
/// spike peak), did an aggressive HL trade PRINT THROUGH a resting maker `level`,
/// in the direction that fills a passive order capturing the reversion? Reuses
/// gapscope's "trade prints through a resting level" idea. DOWN cascade reverts
/// UP: a maker SELL at `level` fills when an aggressive BUY prints at >= level. UP
/// cascade reverts DOWN: a maker BUY at `level` fills on an aggressive SELL <=
/// level. `trades` = (ts, price, buy_aggr) ascending. Pure (only the measured
/// forward window is read; no lookahead beyond the cascade's own horizon).
#[must_use]
fn maker_fills_through(trades: &[(u64, f64, bool)], peak_ts: u64, level: f64, dir_down: bool) -> bool {
    trades.iter().any(|&(ts, px, buy)| {
        ts >= peak_ts && if dir_down { buy && px >= level } else { !buy && px <= level }
    })
}

/// Taker side of the urgent revert EXIT (the maker fade exits by crossing). Used by
/// the Tweak-4 net fee accounting (maker entry + taker exit ~= 6bps round-trip).
const TAKER_EXIT_FEE_BPS: f64 = 4.5;

/// Momentum is a TAKER-in / TAKER-out continuation trade (the entry must CROSS - it is
/// NOT a maker-in-path play), so the round-trip is the full taker fee ~9bps.
const MOM_TAKER_RT_FEE_BPS: f64 = 9.0;

/// Tweak 4 (COMBINED) - the HONEST maker-in-path cascade fade. PROVIDE liquidity INTO
/// the forced flow: rest a maker limit at `arm_px` offset INTO the cascade direction
/// (a BID below market for a DOWN cascade, an ASK above market for an UP cascade).
///
/// FILL RULE (no free fill, no lookahead): the order fills ONLY when the HL tape
/// actually TRADES THROUGH the resting limit in the filling direction at/after
/// `arm_ts` - i.e. an aggressive SELL prints at/below our BID (DOWN) or an aggressive
/// BUY prints at/above our ASK (UP). If the tape never trades through within the
/// cascade horizon -> NO FILL -> None (no trade, no cost). The fill price is the maker
/// LIMIT (`level`) - the aggressor crosses to us, no price improvement.
///
/// Once filled we mark forward to the SAME exit rule as the taker fade (revert target
/// bps via the favorable move OR `max_hold`, then mark-to-market). TRENDERS that fill
/// as price blows through and KEEP GOING against us return a real NEGATIVE capture -
/// every armed+filled cascade contributes its true signed bps (NOT revert-conditioned).
///
/// `hold_gate` = Some((confirm_end_ts, confirm_ok)) wires --ob-confirm as a HOLD/KEEP
/// gate (no-lookahead: the confirm is read at `confirm_end_ts` <= the decision it
/// gates): if the book did NOT confirm a revert by the window-end, a still-resting
/// order is CANCELLED (no fill) and an already-filled position is FLATTENED at the
/// window-end (cut the likely trender early) instead of held to the revert target.
/// Returns (fill_ts, signed_bps) or None if never filled. Pure.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn simulate_maker_fade(
    path: &[(u64, f64)],
    trades: &[(u64, f64, bool)],
    arm_ts: u64,
    arm_px: f64,
    dir_down: bool,
    offset_bps: f64,
    revert_target_bps: f64,
    max_hold_ns: u64,
    hold_gate: Option<(u64, bool)>,
) -> Option<(u64, f64)> {
    if arm_px <= 0.0 || path.is_empty() {
        return None;
    }
    let s = if dir_down { -1.0 } else { 1.0 };
    // resting limit INTO the cascade direction: below market for DOWN, above for UP.
    let level = arm_px * (1.0 + s * offset_bps / 10_000.0);
    if level <= 0.0 {
        return None;
    }
    // FILL: first aggressive print that trades THROUGH the resting limit at/after arm.
    let mut fill_ts: Option<u64> = None;
    for &(ts, px, buy) in trades {
        if ts < arm_ts {
            continue;
        }
        let through = if dir_down { !buy && px <= level } else { buy && px >= level };
        if through {
            fill_ts = Some(ts);
            break;
        }
    }
    let fill_ts = fill_ts?;
    // HOLD-GATE: bail orders the book did not confirm by the window-end.
    let mut force_exit_ts: Option<u64> = None;
    if let Some((ce, confirm_ok)) = hold_gate {
        if !confirm_ok {
            if fill_ts > ce {
                return None; // still resting at the confirm read -> CANCELLED.
            }
            force_exit_ts = Some(ce); // filled before the cut -> flatten at the read.
        }
    }
    // EXIT: identical to the taker fade, entry at the maker LIMIT (level). Favorable
    // = price moves back our way; a trender that runs over returns a real loss.
    let fade_sign = if dir_down { 1.0 } else { -1.0 };
    let mut last_fav = 0.0f64;
    let mut started = false;
    for &(ts, px) in path {
        if ts < fill_ts {
            continue;
        }
        started = true;
        let fav = fade_sign * (px - level) / level * 10_000.0;
        last_fav = fav;
        if fav >= revert_target_bps {
            return Some((fill_ts, fav));
        }
        if let Some(fe) = force_exit_ts {
            if ts >= fe {
                return Some((fill_ts, fav));
            }
        }
        if ts.saturating_sub(fill_ts) >= max_hold_ns {
            return Some((fill_ts, fav));
        }
    }
    if started { Some((fill_ts, last_fav)) } else { Some((fill_ts, 0.0)) }
}

// ----------------------------------------------------------------------------
// configuration + per-cascade record
// ----------------------------------------------------------------------------

#[derive(Clone)]
struct Cfg {
    root: PathBuf,
    coin: String,
    hours: Vec<String>,
    window_ns: u64,
    min_flow: f64,
    cooldown_ns: u64,
    forward_ns: u64,
    delays_ns: Vec<u64>,
    fade_revert_bps: f64,
    fade_hold_ns: u64,
    revert_frac: f64,
    top_n: usize,
    path_sample_ns: u64,
    exhaust: bool,
    exhaust_stall_ns: u64,
    exhaust_decel: f64,
    min_spike_bps: f64,
    min_oidrop_pct: f64,
    ob_confirm: bool,
    confirm_ns: u64,
    confirm_imb: f64,
    maker_fill: bool,
    maker_fee_bps: f64,
    maker_fade: bool,
    maker_offset_bps: f64,
    maker_armgate: bool,
    momentum: bool,
    mom_target_bps: f64,
    mom_stop_bps: f64,
    mom_hold_ns: u64,
    mom_trend: bool,
    mom_trend_max: f64,
}

/// One detected cascade and its characterisation (scalars only; the forward path
/// is transient and dropped after finalisation).
#[derive(Clone)]
struct CascadeStat {
    day: String,
    d_oi: f64,
    p_move: f64,
    fire_ts: u64,
    dir_down: bool,
    oi_drop_pct: f64,
    price_move_bps: f64,
    net_flow: f64,
    spike_bps: f64,
    revert_bps: f64,
    revert_frac: f64,
    reverted: bool,
    full_revert: bool,
    tth_ns: Option<u64>,
    ttf_ns: Option<u64>,
    peak_ns: u64,
    /// bps captured by the reactive fade at each configured delay (None = no entry).
    fades: Vec<Option<f64>>,
    /// True if the cascade reached an exhaustion point (always true when --exhaust
    /// is OFF). When ON and false, the cascade was SKIPPED (no fade entries).
    exhausted: bool,
    /// ns from fire to the exhaustion point (None if never / exhaust off).
    exhaust_ns: Option<u64>,
    /// True if the cascade PASSED the magnitude filter (always true when the
    /// filter is OFF). When false the cascade was magnitude-skipped (no fades).
    passed_mag: bool,
    /// The no-lookahead gate spike used by the filter: |price move from
    /// window-start to fire| in bps (realized at detection, NOT the forward peak).
    gate_spike_bps: f64,
    /// Tweak 3 Part A: did the post-fire book CONFIRM a revert (top-N imbalance
    /// shifted back toward the HIT side within the confirm window)? Always true
    /// when --ob-confirm is OFF (baseline). When false (ob-confirm ON) = skipped.
    confirm_ok: bool,
    /// Top-N depth imbalance at the fire and at the confirm-window-end (anchor).
    imb_start: f64,
    imb_end: f64,
    /// Toward-hit-side imbalance shift over the confirm window (the confirm dial).
    imb_shift: f64,
    /// Tweak 3 Part B (measurement only): would a resting MAKER at the baseline /
    /// post-spike level have FILLED on the way back (tape traded through it)?
    maker_fill_base: bool,
    maker_fill_spike: bool,
    /// Tweak 4 (COMBINED honest maker-in-path fade): was a resting maker order ARMED
    /// for this cascade (true unless an arming-gate skipped a non-confirming cascade)?
    maker_armed: bool,
    /// Did the maker order FILL (HL tape actually traded THROUGH the resting limit)?
    maker_filled: bool,
    /// Signed bps captured by the filled maker fade (entry at the limit, taker exit;
    /// INCLUDES trender losses). None if never filled / not armed.
    maker_capture: Option<f64>,
    /// ns from fire to the maker fill (None if never filled).
    maker_fill_ns: Option<u64>,
    /// MOMENTUM (Tweak 5): signed bps of the trade-WITH-the-continuation TAKER trade at
    /// each configured delay (None = no entry / momentum off / trend-gate skipped).
    moms: Vec<Option<f64>>,
    /// True if the momentum trend-confirm passed (liquidity stayed pulled on the hit
    /// side). Always true when --mom-trend is OFF.
    mom_trend_ok: bool,
}

/// Pending cascade being collected over its forward window.
struct Pending {
    fire_ts: u64,
    dir_down: bool,
    baseline: f64,
    oi_drop_pct: f64,
    price_move_bps: f64,
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

/// Replay one day for ONE (d_oi, p_move) threshold pair; return its cascades.
/// Deterministic + no-lookahead: book/OI/flow folded forward event by event,
/// only `<= now` state read.
fn scan_day(cfg: &Cfg, day: &str, evs: &[LagEvent], d_oi: f64, p_move: f64) -> Vec<CascadeStat> {
    let mut book = OrderBook::with_max_levels(64);
    let mut oi_now = 0.0f64;
    let mut micro_now: Option<f64> = None;

    // detection window buffer: (ts, oi, micro) pushed at path-sample cadence.
    let mut det: VecDeque<(u64, f64, f64)> = VecDeque::new();
    let mut last_det_push = 0u64;
    // signed HL aggressive trade flow within the window: (ts, signed_qty).
    let mut flow: VecDeque<(u64, f64)> = VecDeque::new();

    let mut state = State::Armed;
    let mut blocked_until = 0u64;
    let mut cur = Pending { fire_ts: 0, dir_down: false, baseline: 0.0, oi_drop_pct: 0.0, price_move_bps: 0.0, net_flow: 0.0, end_ts: 0, imb_start: 0.0, imb_end: None };
    let mut cur_path: Vec<(u64, f64)> = Vec::new();
    let mut cur_opath: Vec<(u64, f64)> = Vec::new();
    let mut cur_trades: Vec<(u64, f64, bool)> = Vec::new();
    let mut out: Vec<CascadeStat> = Vec::new();

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
                    if (cfg.maker_fill || cfg.maker_fade) && matches!(state, State::Collecting) {
                        cur_trades.push((now, ev.price.to_f64(), aggr == Side::Bid));
                    }
                }
            }
            (_, LagKind::OpenInterest) => oi_now = ev.aux,
            _ => {}
        }

        let wlo = now.saturating_sub(cfg.window_ns);
        while let Some(&(ts, _)) = flow.front() {
            if ts < wlo { flow.pop_front(); } else { break; }
        }
        if let Some(m) = micro_now {
            if det.is_empty() || now.saturating_sub(last_det_push) >= cfg.path_sample_ns {
                det.push_back((now, oi_now, m));
                last_det_push = now;
            }
        }
        while let Some(&(ts, _, _)) = det.front() {
            if ts < wlo { det.pop_front(); } else { break; }
        }

        match state {
            State::Collecting => {
                if let Some(m) = micro_now {
                    let push = match cur_path.last() {
                        Some(&(t, _)) => now.saturating_sub(t) >= cfg.path_sample_ns,
                        None => true,
                    };
                    if push {
                        cur_path.push((now, m));
                        cur_opath.push((now, oi_now));
                    }
                }
                if cur.imb_end.is_none() && now >= cur.fire_ts.saturating_add(cfg.confirm_ns) {
                    let (bd, ad) = top_depth(&book, cfg.top_n);
                    cur.imb_end = Some(imbalance(bd, ad));
                }
                if now >= cur.end_ts {
                    out.push(finalize(&cur, &cur_path, &cur_opath, &cur_trades, cfg, day, d_oi, p_move));
                    cur_path.clear();
                    cur_opath.clear();
                    cur_trades.clear();
                    state = State::Armed;
                }
            }
            State::Armed => {
                if now >= blocked_until {
                    if let (Some(&(fts, oi_s, mic_s)), Some(mic_n)) = (det.front(), micro_now) {
                        let span = now.saturating_sub(fts);
                        if oi_s > 0.0 && mic_s > 0.0 && span >= cfg.window_ns / 2 {
                            let oi_drop = (oi_s - oi_now) / oi_s * 100.0;
                            let pm = (mic_n - mic_s) / mic_s * 10_000.0;
                            let net: f64 = flow.iter().map(|&(_, q)| q).sum();
                            let down = pm <= -p_move && net <= -cfg.min_flow && oi_drop >= d_oi;
                            let up = pm >= p_move && net >= cfg.min_flow && oi_drop >= d_oi;
                            if down || up {
                                cur = Pending {
                                    fire_ts: now,
                                    dir_down: down,
                                    baseline: mic_s,
                                    oi_drop_pct: oi_drop,
                                    price_move_bps: pm,
                                    net_flow: net,
                                    end_ts: now.saturating_add(cfg.forward_ns),
                                    imb_start: {
                                        let (bd, ad) = top_depth(&book, cfg.top_n);
                                        imbalance(bd, ad)
                                    },
                                    imb_end: None,
                                };
                                cur_path.clear();
                                cur_opath.clear();
                                cur_trades.clear();
                                cur_path.push((now, mic_n));
                                cur_opath.push((now, oi_now));
                                blocked_until = now.saturating_add(cfg.cooldown_ns);
                                state = State::Collecting;
                            }
                        }
                    }
                }
            }
        }
    }
    // a cascade still collecting at end-of-day is dropped (incomplete horizon).
    out
}

/// Finalise a collected cascade into scalar statistics.
#[allow(clippy::too_many_arguments)]
fn finalize(cur: &Pending, path: &[(u64, f64)], oi_path: &[(u64, f64)], trades: &[(u64, f64, bool)], cfg: &Cfg, day: &str, d_oi: f64, p_move: f64) -> CascadeStat {
    let rev = measure_reversion(path, cur.fire_ts, cur.baseline, cur.dir_down);
    let reverted = rev.revert_frac >= cfg.revert_frac;
    // Choose the fade ENTRY ANCHOR: baseline fades at the fire; --exhaust waits for
    // the exhaustion point (forced flow runs out) and only then fades. If exhaustion
    // never fires within the horizon, the cascade is SKIPPED (no entries).
    //
    // MAGNITUDE FILTER (Tweak 2, default OFF). Gate on info realized AT/before the
    // entry decision ONLY: the detection-window move (window-start -> fire) and the
    // OI-drop% - both known at the fire. NEVER the forward peak (lookahead). A
    // filtered cascade is still detected + characterised but yields NO fade entry.
    let gate_spike_bps = cur.price_move_bps.abs();
    let passed_mag = passes_magnitude(gate_spike_bps, cur.oi_drop_pct, cfg.min_spike_bps, cfg.min_oidrop_pct);
    // Tweak 3 Part A: order-book REVERT-vs-TREND confirm (NO-LOOKAHEAD). imb_start
    // is read at the fire, imb_end at the confirm-window-end (= the entry anchor);
    // both are <= the entry time, so the gate uses ONLY at/before-entry book state.
    let imb_start = cur.imb_start;
    let imb_end = cur.imb_end.unwrap_or(imb_start);
    let imb_shift = if cur.dir_down { imb_end - imb_start } else { imb_start - imb_end };
    let confirm_ok = !cfg.ob_confirm || confirm_revert(imb_start, imb_end, cur.dir_down, cfg.confirm_imb);
    // Entry anchor: magnitude + confirm gates first (fail => no entry). With
    // --ob-confirm we DELAY entry to the confirm-window-end (latency is slack) so
    // the same delays (0/800/2000ms) are applied AFTER the confirm read.
    let exhaust_ts = if !passed_mag || !confirm_ok {
        None
    } else if cfg.exhaust {
        find_exhaustion(path, oi_path, cur.fire_ts, cur.dir_down, cfg.exhaust_stall_ns, cfg.exhaust_decel)
    } else if cfg.ob_confirm {
        Some(cur.fire_ts.saturating_add(cfg.confirm_ns))
    } else {
        Some(cur.fire_ts)
    };
    // Tweak 3 Part B (measurement only; NEVER gates trading): would a resting MAKER
    // have FILLED on the way back? "tape trades through a resting level" reused from
    // gapscope. peak = spike extreme; levels = pre-cascade baseline / post-spike.
    let peak_ts = cur.fire_ts.saturating_add(rev.peak_ns);
    let s_dir = if cur.dir_down { -1.0 } else { 1.0 };
    let extreme_px = cur.baseline * (1.0 + s_dir * rev.spike_bps / 10_000.0);
    let (maker_fill_base, maker_fill_spike) = if cfg.maker_fill {
        (
            maker_fills_through(trades, peak_ts, cur.baseline, cur.dir_down),
            maker_fills_through(trades, peak_ts, extreme_px, cur.dir_down),
        )
    } else {
        (false, false)
    };
    // Tweak 4 (COMBINED, default OFF): the HONEST maker-in-path cascade fade. Rest a
    // maker limit INTO the forced flow at the FIRE; it FILLS only when the HL tape
    // actually trades THROUGH the limit (no free fill, no lookahead). Once filled, mark
    // forward to the SAME exit as the taker fade so TRENDERS that fill and run over
    // count their real loss. --ob-confirm manages the order: default = HOLD-GATE (cut a
    // position the book did not confirm by the window-end); --maker-armgate = only arm
    // AFTER a confirm. Both read the confirm <= the decision it gates (no-lookahead).
    let (maker_armed, maker_filled, maker_capture, maker_fill_ns) = if cfg.maker_fade && passed_mag {
        // (passed_mag gates the maker arm too: --min-spike/--min-oidrop filter on the
        // realized window move, no-lookahead; a no-op when those dials are unset.)
        let arm_px_fire = path.first().map_or(0.0, |&(_, p)| p);
        if cfg.ob_confirm && cfg.maker_armgate {
            // ARMING-GATE alt: arm only after a confirm, at the confirm-window-end.
            if confirm_ok {
                let ce = cur.fire_ts.saturating_add(cfg.confirm_ns);
                let arm_px = path.iter().find(|&&(ts, _)| ts >= ce).map_or(arm_px_fire, |&(_, p)| p);
                match simulate_maker_fade(path, trades, ce, arm_px, cur.dir_down, cfg.maker_offset_bps, cfg.fade_revert_bps, cfg.fade_hold_ns, None) {
                    Some((fts, bps)) => (true, true, Some(bps), Some(fts.saturating_sub(cur.fire_ts))),
                    None => (true, false, None, None),
                }
            } else {
                (false, false, None, None)
            }
        } else {
            // HOLD-GATE (with --ob-confirm) or UNGATED (no --ob-confirm): arm at the fire.
            let hold_gate = if cfg.ob_confirm {
                Some((cur.fire_ts.saturating_add(cfg.confirm_ns), confirm_ok))
            } else {
                None
            };
            match simulate_maker_fade(path, trades, cur.fire_ts, arm_px_fire, cur.dir_down, cfg.maker_offset_bps, cfg.fade_revert_bps, cfg.fade_hold_ns, hold_gate) {
                Some((fts, bps)) => (true, true, Some(bps), Some(fts.saturating_sub(cur.fire_ts))),
                None => (true, false, None, None),
            }
        }
    } else {
        (false, false, None, None)
    };
    // MOMENTUM (Tweak 5, default OFF): trade WITH the cascade continuation. The
    // optional trend-confirm is the MIRROR of ob-confirm (liquidity STAYS PULLED on the
    // hit side => imbalance does NOT return). No-lookahead: imb_end was read at the
    // confirm-window-end; momentum entries enter at fire+delay and the forward path
    // decides target/stop by first-touch. Independent of the fade/maker tweaks.
    let mom_trend_ok = if cfg.momentum && cfg.mom_trend {
        confirm_trend(imb_start, imb_end, cur.dir_down, cfg.mom_trend_max)
    } else {
        true
    };
    let moms: Vec<Option<f64>> = if cfg.momentum && mom_trend_ok {
        cfg.delays_ns
            .iter()
            .map(|&d| simulate_momentum(path, cur.fire_ts, cur.dir_down, d, cfg.mom_target_bps, cfg.mom_stop_bps, cfg.mom_hold_ns))
            .collect()
    } else {
        cfg.delays_ns.iter().map(|_| None).collect()
    };
    let fades: Vec<Option<f64>> = match exhaust_ts {
        Some(anchor) => cfg
            .delays_ns
            .iter()
            .map(|&d| simulate_fade(path, anchor, cur.dir_down, d, cfg.fade_revert_bps, cfg.fade_hold_ns))
            .collect(),
        None => cfg.delays_ns.iter().map(|_| None).collect(),
    };
    let exhausted = exhaust_ts.is_some();
    let exhaust_ns = if cfg.exhaust { exhaust_ts.map(|tt| tt.saturating_sub(cur.fire_ts)) } else { None };
    CascadeStat {
        day: day.to_string(),
        d_oi,
        p_move,
        fire_ts: cur.fire_ts,
        dir_down: cur.dir_down,
        oi_drop_pct: cur.oi_drop_pct,
        price_move_bps: cur.price_move_bps,
        net_flow: cur.net_flow,
        spike_bps: rev.spike_bps,
        revert_bps: rev.revert_bps,
        revert_frac: rev.revert_frac,
        reverted,
        full_revert: rev.full_revert,
        tth_ns: rev.tth_ns,
        ttf_ns: rev.ttf_ns,
        peak_ns: rev.peak_ns,
        fades,
        exhausted,
        exhaust_ns,
        passed_mag,
        gate_spike_bps,
        confirm_ok,
        imb_start,
        imb_end,
        imb_shift,
        maker_fill_base,
        maker_fill_spike,
        maker_armed,
        maker_filled,
        maker_capture,
        maker_fill_ns,
        moms,
        mom_trend_ok,
    }
}
// ----------------------------------------------------------------------------
// aggregate reporting
// ----------------------------------------------------------------------------

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.0}s", ns as f64 / 1e9)
    } else {
        format!("{}ms", ns / 1_000_000)
    }
}

/// Collected fade captures (bps) at one delay index across cascades (Some only).
fn fade_col(ds: &[&CascadeStat], di: usize) -> Vec<f64> {
    ds.iter().filter_map(|d| d.fades.get(di).copied().flatten()).collect()
}

/// Collected MOMENTUM captures (bps) at one delay index across cascades (Some only).
fn mom_col(ds: &[&CascadeStat], di: usize) -> Vec<f64> {
    ds.iter().filter_map(|d| d.moms.get(di).copied().flatten()).collect()
}

/// Compact one-line per-day summary.
fn report_day_line(day: &str, ds: &[&CascadeStat], cfg: &Cfg) {
    let n = ds.len();
    if n == 0 {
        println!("  {day}   cascades 0");
        return;
    }
    let rev = ds.iter().filter(|d| d.reverted).count();
    let spike: Vec<f64> = ds.iter().map(|d| d.spike_bps).collect();
    let mut fade_str = String::new();
    for (di, &dl) in cfg.delays_ns.iter().enumerate() {
        let v = fade_col(ds, di);
        fade_str.push_str(&format!("  fade@{}={:+.1}", fmt_ns(dl), mean(&v)));
    }
    println!(
        "  {day}   cascades {n}  spike {:.1}bps  revert {:.0}%{}",
        mean(&spike),
        rev as f64 / n as f64 * 100.0,
        fade_str
    );
}

/// Full pooled report block.
fn report_pooled(label: &str, ds: &[&CascadeStat], cfg: &Cfg, num_days: usize) {
    println!("\n---------- {label} ----------");
    let n = ds.len();
    let per_day = if num_days > 0 { n as f64 / num_days as f64 } else { 0.0 };
    println!("cascades {n}   ({per_day:.1}/day over {num_days} days)");
    if n == 0 {
        println!("  (none detected - thresholds too strict or quiet regime)");
        return;
    }
    if cfg.exhaust {
        let ex = ds.iter().filter(|d| d.exhausted).count();
        let exn: Vec<f64> = ds.iter().filter_map(|d| d.exhaust_ns).map(|x| x as f64 / 1e9).collect();
        println!(
            "EXHAUSTION (entry-conditioned)  fired {ex}/{n} = {:.0}%   skipped {}   median wait {:.1}s",
            ex as f64 / n as f64 * 100.0,
            n - ex,
            if exn.is_empty() { 0.0 } else { median(&exn) }
        );
    }
    if cfg.min_spike_bps > 0.0 || cfg.min_oidrop_pct > 0.0 {
        let pass = ds.iter().filter(|d| d.passed_mag).count();
        let gs: Vec<f64> = ds.iter().filter(|d| d.passed_mag).map(|d| d.gate_spike_bps).collect();
        println!(
            "MAGNITUDE FILTER (min-spike {:.0}bps  min-oidrop {:.2}%)  passed {pass}/{n} = {:.0}%   gate-spike(passed) mean {:.1}bps",
            cfg.min_spike_bps, cfg.min_oidrop_pct,
            pass as f64 / n as f64 * 100.0,
            if gs.is_empty() { 0.0 } else { mean(&gs) }
        );
    }
    if cfg.ob_confirm {
        let pass = ds.iter().filter(|d| d.confirm_ok).count();
        let sh: Vec<f64> = ds.iter().map(|d| d.imb_shift).collect();
        let shp: Vec<f64> = ds.iter().filter(|d| d.confirm_ok).map(|d| d.imb_shift).collect();
        println!(
            "OB-CONFIRM (window {}  min-imb-shift {:.2})  confirmed {pass}/{n} = {:.0}%   skipped {}   imb-shift mean(all) {:+.3}  mean(confirmed) {:+.3}",
            fmt_ns(cfg.confirm_ns), cfg.confirm_imb,
            pass as f64 / n as f64 * 100.0,
            n - pass,
            mean(&sh),
            if shp.is_empty() { 0.0 } else { mean(&shp) }
        );
    }
    if cfg.maker_fill {
        let revd: Vec<&CascadeStat> = ds.iter().copied().filter(|d| d.reverted).collect();
        let nr = revd.len();
        if nr > 0 {
            let fb = revd.iter().filter(|d| d.maker_fill_base).count();
            let fsp = revd.iter().filter(|d| d.maker_fill_spike).count();
            let gross: Vec<f64> = revd.iter().map(|d| d.revert_bps).collect();
            let g = mean(&gross);
            println!(
                "MAKER-FILL (Part B, measurement only; reverted n={nr})  baseline-level filled {fb}/{nr} = {:.0}%   post-spike-level filled {fsp}/{nr} = {:.0}%",
                fb as f64 / nr as f64 * 100.0,
                fsp as f64 / nr as f64 * 100.0
            );
            println!(
                "  reversion capture (gross) mean {:+.2}bps   NET maker(-{:.1}) {:+.2}bps   vs NET taker(-9) {:+.2}bps",
                g, cfg.maker_fee_bps, g - cfg.maker_fee_bps, g - 9.0
            );
        } else {
            println!("MAKER-FILL (Part B)  no reverted cascades to measure");
        }
    }
    if cfg.maker_fade {
        let armed = ds.iter().filter(|d| d.maker_armed).count();
        let filled = ds.iter().filter(|d| d.maker_filled).count();
        let caps: Vec<f64> = ds.iter().filter_map(|d| d.maker_capture).collect();
        let fillms: Vec<f64> = ds.iter().filter_map(|d| d.maker_fill_ns).map(|x| x as f64 / 1e6).collect();
        println!(
            "MAKER-FADE (Tweak 4 COMBINED: honest maker-in-path entry + taker exit; offset {:.0}bps)",
            cfg.maker_offset_bps
        );
        println!(
            "  armed {armed}/{n}   filled {filled}  (fill rate {:.0}% of armed)   median fill-wait {:.0}ms",
            if armed > 0 { filled as f64 / armed as f64 * 100.0 } else { 0.0 },
            if fillms.is_empty() { 0.0 } else { median(&fillms) }
        );
        if filled == 0 {
            println!("  (no fills - the tape never traded through the resting limit)");
        } else {
            let g = mean(&caps);
            let wins = caps.iter().filter(|&&x| x > 0.0).count();
            let worst = caps.iter().copied().fold(f64::INFINITY, f64::min);
            let tail = caps.iter().filter(|&&x| x < -30.0).count();
            let net6 = g - cfg.maker_fee_bps - TAKER_EXIT_FEE_BPS;
            let net3 = g - 2.0 * cfg.maker_fee_bps;
            println!(
                "  GROSS (incl trenders)  mean {:+.2}bps  median {:+.2}  t-stat {:+.2}  win {:.0}%  worst {:+.1}  <-30bps {:.0}%  n={}",
                g, median(&caps), tstat(&caps), wins as f64 / filled as f64 * 100.0, worst,
                tail as f64 / filled as f64 * 100.0, filled
            );
            println!(
                "  NET maker(-{:.1})+taker(-{:.1}) = -{:.1}bps RT -> {:+.2}bps   [alt both-maker -{:.1}bps RT -> {:+.2}bps]",
                cfg.maker_fee_bps, TAKER_EXIT_FEE_BPS, cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS, net6,
                2.0 * cfg.maker_fee_bps, net3
            );
        }
    }
    let down = ds.iter().filter(|d| d.dir_down).count();
    println!("direction                     DOWN {down}  UP {}", n - down);

    let spike: Vec<f64> = ds.iter().map(|d| d.spike_bps).collect();
    let oidrop: Vec<f64> = ds.iter().map(|d| d.oi_drop_pct).collect();
    println!(
        "SPIKE size                    mean {:.1}bps  median {:.1}bps   (OI-drop mean {:.2}%)",
        mean(&spike), median(&spike), mean(&oidrop)
    );

    let rev = ds.iter().filter(|d| d.reverted).count();
    let full = ds.iter().filter(|d| d.full_revert).count();
    let revbps: Vec<f64> = ds.iter().map(|d| d.revert_bps).collect();
    let revfrac: Vec<f64> = ds.iter().map(|d| d.revert_frac).collect();
    println!(
        "REVERT vs CONTINUE            revert(>={:.0}% of spike) {}/{n} = {:.0}%   continue/trend {:.0}%",
        cfg.revert_frac * 100.0,
        rev,
        rev as f64 / n as f64 * 100.0,
        (n - rev) as f64 / n as f64 * 100.0
    );
    println!(
        "  reversion magnitude         mean {:.1}bps  (mean frac of spike {:.2})   full-revert-to-baseline {:.0}%",
        mean(&revbps), mean(&revfrac), full as f64 / n as f64 * 100.0
    );
    let tth: Vec<f64> = ds.iter().filter_map(|d| d.tth_ns).map(|x| x as f64 / 1e3).collect();
    let ttf: Vec<f64> = ds.iter().filter_map(|d| d.ttf_ns).map(|x| x as f64 / 1e3).collect();
    if !tth.is_empty() {
        println!("  time-to-HALF-revert         median {:.0}ms  (n={})", median(&tth) / 1e3, tth.len());
    }
    if !ttf.is_empty() {
        println!("  time-to-FULL-revert         median {:.0}ms  (n={})", median(&ttf) / 1e3, ttf.len());
    }

    println!("REACTIVE FADE captured bps (the tradeability test):");
    for (di, &dl) in cfg.delays_ns.iter().enumerate() {
        let v = fade_col(ds, di);
        if v.is_empty() {
            println!("  delay {:>6}              (no entries)", fmt_ns(dl));
            continue;
        }
        let wins = v.iter().filter(|&&x| x > 0.0).count();
        let worst = v.iter().copied().fold(f64::INFINITY, f64::min);
        let tail = v.iter().filter(|&&x| x < -30.0).count();
        println!(
            "  delay {:>6}   mean {:+.2}bps  median {:+.2}  t-stat {:+.2}  win {:.0}%  worst {:+.1}  <-30bps {:.0}%  n={}",
            fmt_ns(dl),
            mean(&v),
            median(&v),
            tstat(&v),
            wins as f64 / v.len() as f64 * 100.0,
            worst,
            tail as f64 / v.len() as f64 * 100.0,
            v.len()
        );
    }

    if cfg.momentum {
        if cfg.mom_trend {
            let pass = ds.iter().filter(|d| d.mom_trend_ok).count();
            println!(
                "MOM TREND-CONFIRM (liquidity STAYS PULLED on hit side: toward-hit imbalance return <= {:.2} within {})  confirmed {pass}/{n} = {:.0}%   skipped {}",
                cfg.mom_trend_max, fmt_ns(cfg.confirm_ns),
                pass as f64 / n as f64 * 100.0,
                n - pass
            );
        }
        println!(
            "MOMENTUM (Tweak 5: trade WITH the continuation; DOWN->short UP->long; target {:.0}bps / stop {:.0}bps / hold {}; TAKER round-trip ~{:.0}bps):",
            cfg.mom_target_bps, cfg.mom_stop_bps, fmt_ns(cfg.mom_hold_ns), MOM_TAKER_RT_FEE_BPS
        );
        for (di, &dl) in cfg.delays_ns.iter().enumerate() {
            let v = mom_col(ds, di);
            if v.is_empty() {
                println!("  delay {:>6}              (no entries)", fmt_ns(dl));
                continue;
            }
            let wins = v.iter().filter(|&&x| x > 0.0).count();
            let worst = v.iter().copied().fold(f64::INFINITY, f64::min);
            let g = mean(&v);
            println!(
                "  delay {:>6}   GROSS mean {:+.2}bps  median {:+.2}  t-stat {:+.2}  win {:.0}%  worst {:+.1}  n={}   NET(-{:.0}) {:+.2}bps",
                fmt_ns(dl), g, median(&v), tstat(&v),
                wins as f64 / v.len() as f64 * 100.0, worst, v.len(),
                MOM_TAKER_RT_FEE_BPS, g - MOM_TAKER_RT_FEE_BPS
            );
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
        window_ns: 5_000_000_000,
        min_flow: 0.0,
        cooldown_ns: 30_000_000_000,
        forward_ns: 60_000_000_000,
        delays_ns: vec![0, 800_000_000, 2_000_000_000],
        fade_revert_bps: 10.0,
        fade_hold_ns: 30_000_000_000,
        revert_frac: 0.5,
        top_n: 5,
        path_sample_ns: 50_000_000,
        exhaust: false,
        exhaust_stall_ns: 2_000_000_000,
        exhaust_decel: 0.5,
        min_spike_bps: 0.0,
        min_oidrop_pct: 0.0,
        ob_confirm: false,
        confirm_ns: 800_000_000,
        confirm_imb: 0.10,
        maker_fill: false,
        maker_fee_bps: 1.5,
        maker_fade: false,
        maker_offset_bps: 0.0,
        maker_armgate: false,
        momentum: false,
        mom_target_bps: 25.0,
        mom_stop_bps: 12.0,
        mom_hold_ns: 30_000_000_000,
        mom_trend: false,
        mom_trend_max: 0.0,
    };
    let mut dates: Vec<String> = Vec::new();
    let mut oi_drops: Vec<f64> = vec![0.5];
    let mut price_moves: Vec<f64> = vec![10.0];
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
            "--window" => cfg.window_ns = parse_dur(&val()?)?,
            "--oi-drop" => oi_drops = parse_f64_list(&val()?)?,
            "--price-move" => price_moves = parse_f64_list(&val()?)?,
            "--min-flow" => cfg.min_flow = val()?.parse().map_err(|e| format!("min-flow: {e}"))?,
            "--cooldown" => cfg.cooldown_ns = parse_dur(&val()?)?,
            "--forward" => cfg.forward_ns = parse_dur(&val()?)?,
            "--delays" => {
                cfg.delays_ns = val()?.split(',').map(|s| parse_dur(s.trim())).collect::<Result<Vec<_>, _>>()?;
            }
            "--fade-revert" => cfg.fade_revert_bps = val()?.parse().map_err(|e| format!("fade-revert: {e}"))?,
            "--fade-hold" => cfg.fade_hold_ns = parse_dur(&val()?)?,
            "--revert-frac" => cfg.revert_frac = val()?.parse().map_err(|e| format!("revert-frac: {e}"))?,
            "--top" => cfg.top_n = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--sample" => cfg.path_sample_ns = parse_dur(&val()?)?,
            "--exhaust" => cfg.exhaust = true,
            "--exhaust-stall" => cfg.exhaust_stall_ns = parse_dur(&val()?)?,
            "--exhaust-decel" => cfg.exhaust_decel = val()?.parse().map_err(|e| format!("exhaust-decel: {e}"))?,
            "--min-spike" => cfg.min_spike_bps = val()?.parse().map_err(|e| format!("min-spike: {e}"))?,
            "--min-oidrop" => cfg.min_oidrop_pct = val()?.parse().map_err(|e| format!("min-oidrop: {e}"))?,
            "--ob-confirm" => cfg.ob_confirm = true,
            "--ob-confirm-window" => cfg.confirm_ns = parse_dur(&val()?)?,
            "--ob-confirm-imb" => cfg.confirm_imb = val()?.parse().map_err(|e| format!("ob-confirm-imb: {e}"))?,
            "--maker-fill" => cfg.maker_fill = true,
            "--maker-fee" => cfg.maker_fee_bps = val()?.parse().map_err(|e| format!("maker-fee: {e}"))?,
            "--maker-fade" => cfg.maker_fade = true,
            "--maker-offset" => cfg.maker_offset_bps = val()?.parse().map_err(|e| format!("maker-offset: {e}"))?,
            "--maker-armgate" => cfg.maker_armgate = true,
            "--momentum" => cfg.momentum = true,
            "--mom-target" => cfg.mom_target_bps = val()?.parse().map_err(|e| format!("mom-target: {e}"))?,
            "--mom-stop" => cfg.mom_stop_bps = val()?.parse().map_err(|e| format!("mom-stop: {e}"))?,
            "--mom-hold" => cfg.mom_hold_ns = parse_dur(&val()?)?,
            "--mom-trend" => cfg.mom_trend = true,
            "--mom-trend-max" => cfg.mom_trend_max = val()?.parse().map_err(|e| format!("mom-trend-max: {e}"))?,
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
    if cfg.delays_ns.is_empty() {
        return Err("--delays must have >= 1 entry".into());
    }

    println!("oiscope - HL OI-drop forced-flow cascade study (analysis only, no trading)");
    println!("coin {}  dates {}", cfg.coin, dates.join(","));
    println!(
        "window {}  cooldown {}  forward {}  top {}  sample {}  min-flow {}",
        fmt_ns(cfg.window_ns), fmt_ns(cfg.cooldown_ns), fmt_ns(cfg.forward_ns), cfg.top_n, fmt_ns(cfg.path_sample_ns), cfg.min_flow
    );
    let delay_lbl: Vec<String> = cfg.delays_ns.iter().map(|&d| fmt_ns(d)).collect();
    println!(
        "fade: enter at delays [{}]  exit on {:.0}bps revert or {} hold   revert-class >={:.0}% of spike",
        delay_lbl.join(","), cfg.fade_revert_bps, fmt_ns(cfg.fade_hold_ns), cfg.revert_frac * 100.0
    );
    if cfg.exhaust {
        println!(
            "EXHAUSTION ON: fade only after OI-bleed decel <= {:.2}x peak AND no new price extreme for {} (entry = exhaustion point + delays)",
            cfg.exhaust_decel, fmt_ns(cfg.exhaust_stall_ns)
        );
    } else {
        println!("EXHAUSTION OFF (baseline): fade immediately at fire + delays");
    }
    if cfg.min_spike_bps > 0.0 || cfg.min_oidrop_pct > 0.0 {
        println!(
            "MAGNITUDE FILTER ON: fade only cascades with realized detection move >= {:.0}bps AND OI-drop >= {:.2}% (no-lookahead: gate = window-start->fire move, NOT forward peak)",
            cfg.min_spike_bps, cfg.min_oidrop_pct
        );
    } else {
        println!("MAGNITUDE FILTER OFF (baseline): fade every detected cascade");
    }
    if cfg.ob_confirm {
        println!(
            "OB-CONFIRM ON (Tweak 3 Part A): fade only if top-{} depth imbalance shifts >= {:.2} back toward the HIT side within {} of the fire; entry DELAYED to confirm-window-end + delays (no-lookahead: book read <= entry). Non-confirming cascades SKIPPED.",
            cfg.top_n, cfg.confirm_imb, fmt_ns(cfg.confirm_ns)
        );
    } else {
        println!("OB-CONFIRM OFF (baseline): no order-book revert gate");
    }
    if cfg.maker_fill {
        println!(
            "MAKER-FILL MEASURE ON (Tweak 3 Part B): for reverted cascades, report whether the HL tape traded THROUGH the baseline / post-spike level (maker-fillable) + maker-fee(-{:.1}bps)-adjusted capture. Measurement only, never gates trading.",
            cfg.maker_fee_bps
        );
    }
    if cfg.maker_fade {
        let gate = if cfg.ob_confirm && cfg.maker_armgate {
            "ob-confirm = ARMING gate (arm only after a confirm, at window-end)"
        } else if cfg.ob_confirm {
            "ob-confirm = HOLD gate (arm at fire; flatten/cancel positions the book did NOT confirm by window-end)"
        } else {
            "ungated (arm at fire, no ob-confirm)"
        };
        println!(
            "MAKER-FADE ON (Tweak 4 COMBINED, honest): rest a maker limit INTO the forced flow at offset {:.0}bps (bid below for DOWN / ask above for UP); FILL only when the HL tape trades THROUGH it (no free fill, no lookahead); on fill, ride to the SAME exit ({:.0}bps revert / {} hold) - TRENDERS that fill and run over count their REAL loss. Fees: maker entry -{:.1}bps + taker exit -{:.1}bps = -{:.1}bps RT (alt both-maker -{:.1}bps). Gate: {}.",
            cfg.maker_offset_bps, cfg.fade_revert_bps, fmt_ns(cfg.fade_hold_ns),
            cfg.maker_fee_bps, TAKER_EXIT_FEE_BPS, cfg.maker_fee_bps + TAKER_EXIT_FEE_BPS, 2.0 * cfg.maker_fee_bps, gate
        );
    }
    if cfg.momentum {
        let gate = if cfg.mom_trend {
            format!("trend-confirm ON (liquidity STAYS pulled: toward-hit imbalance return <= {:.2} within {})", cfg.mom_trend_max, fmt_ns(cfg.confirm_ns))
        } else {
            "ungated (every detected cascade)".to_string()
        };
        println!(
            "MOMENTUM ON (Tweak 5, trade WITH the continuation): DOWN cascade -> SELL/short, UP -> BUY/long; enter TAKER at fire + delays [{}]; FIRST-TOUCH exit on +{:.0}bps target OR -{:.0}bps stop OR {} hold; TAKER round-trip ~{:.0}bps (GROSS + NET-of-{:.0} reported). Gate: {}. Reverters that stop out ARE counted (no conditioning).",
            delay_lbl.join(","), cfg.mom_target_bps, cfg.mom_stop_bps, fmt_ns(cfg.mom_hold_ns), MOM_TAKER_RT_FEE_BPS, MOM_TAKER_RT_FEE_BPS, gate
        );
    }
    println!("SWEEP oi-drop {:?}%  x  price-move {:?}bps", oi_drops, price_moves);

    // build the threshold grid (knob-bite: cascade set must move with these).
    let mut combos: Vec<(f64, f64)> = Vec::new();
    for &d in &oi_drops {
        for &p in &price_moves {
            combos.push((d, p));
        }
    }
    let mut pooled: Vec<Vec<CascadeStat>> = vec![Vec::new(); combos.len()];

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
        for (ci, &(d, p)) in combos.iter().enumerate() {
            let st = scan_day(&cfg, day, &evs, d, p);
            pooled[ci].extend(st);
        }
        loaded_days.push(day.clone());
    }
    let num_days = loaded_days.len();

    for (ci, &(d, p)) in combos.iter().enumerate() {
        println!("\n==================== oi-drop>={d}%  price-move>={p}bps ====================");
        for day in &loaded_days {
            let sub: Vec<&CascadeStat> = pooled[ci].iter().filter(|s| &s.day == day).collect();
            report_day_line(day, &sub, &cfg);
        }
        let all: Vec<&CascadeStat> = pooled[ci].iter().collect();
        report_pooled("POOLED (all days)", &all, &cfg, num_days);
    }

    if let Some(path) = &dump {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| format!("dump: {e}"))?;
        let mut hdr = String::from("day,oi_drop_thr,price_move_thr,fire_ts,dir_down,oi_drop_pct,price_move_bps,net_flow,spike_bps,revert_bps,revert_frac,reverted,full_revert,exhausted,exhaust_ms,tth_ms,ttf_ms,peak_ms,passed_mag,gate_spike_bps,confirm_ok,imb_start,imb_end,imb_shift,maker_fill_base,maker_fill_spike");
        for &dl in &cfg.delays_ns {
            hdr.push_str(&format!(",fade_{}", fmt_ns(dl)));
        }
        hdr.push_str(",mom_trend_ok");
        for &dl in &cfg.delays_ns {
            hdr.push_str(&format!(",mom_{}", fmt_ns(dl)));
        }
        writeln!(f, "{hdr}").map_err(|e| format!("dump: {e}"))?;
        for col in &pooled {
            for c in col {
                let mut line = format!(
                    "{},{},{},{},{},{:.4},{:.3},{:.4},{:.3},{:.3},{:.4},{},{},{},{},{},{},{:.0},{},{:.3},{},{:.4},{:.4},{:.4},{},{}",
                    c.day, c.d_oi, c.p_move, c.fire_ts, c.dir_down, c.oi_drop_pct, c.price_move_bps, c.net_flow,
                    c.spike_bps, c.revert_bps, c.revert_frac, c.reverted, c.full_revert,
                    c.exhausted,
                    c.exhaust_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.tth_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.ttf_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.peak_ns as f64 / 1e6,
                    c.passed_mag,
                    c.gate_spike_bps,
                    c.confirm_ok,
                    c.imb_start,
                    c.imb_end,
                    c.imb_shift,
                    c.maker_fill_base,
                    c.maker_fill_spike
                );
                for fv in &c.fades {
                    match fv {
                        Some(x) => line.push_str(&format!(",{x:.3}")),
                        None => line.push_str(",NA"),
                    }
                }
                line.push_str(&format!(",{}", c.mom_trend_ok));
                for mv in &c.moms {
                    match mv {
                        Some(x) => line.push_str(&format!(",{x:.3}")),
                        None => line.push_str(",NA"),
                    }
                }
                writeln!(f, "{line}").map_err(|e| format!("dump: {e}"))?;
            }
        }
        let total: usize = pooled.iter().map(Vec::len).sum();
        eprintln!("dumped {total} cascade rows to {path}");
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
        assert_eq!(parse_dur("800ms").unwrap(), 800_000_000);
        assert_eq!(parse_dur("2s").unwrap(), 2_000_000_000);
        assert_eq!(parse_dur("0").unwrap(), 0);
        assert!(parse_dur("-1s").is_err());
    }

    #[test]
    fn tstat_zero_and_positive() {
        assert!(tstat(&[1.0]).abs() < 1e-12, "n<2 => 0");
        // constant series => zero variance => 0 (no significance).
        assert!(tstat(&[5.0, 5.0, 5.0]).abs() < 1e-12);
        // clearly-positive series => positive t.
        assert!(tstat(&[2.0, 3.0, 2.5, 3.5, 2.0]) > 2.0);
    }

    #[test]
    fn reversion_full_bounce_back() {
        // DOWN cascade: baseline 100, drops to 99 (=100bps spike), returns to 100.
        let base = 100.0;
        let path = vec![(0u64, 100.0), (1_000_000_000, 99.5), (2_000_000_000, 99.0), (3_000_000_000, 99.5), (4_000_000_000, 100.0)];
        let r = measure_reversion(&path, 0, base, true);
        assert!((r.spike_bps - 100.0).abs() < 1.0, "spike ~100bps, got {}", r.spike_bps);
        assert!(r.full_revert, "should fully revert to baseline");
        assert!(r.revert_frac >= 0.99, "frac ~1.0, got {}", r.revert_frac);
        assert!(r.ttf_ns.is_some());
    }

    #[test]
    fn reversion_trend_does_not_revert() {
        // DOWN cascade that keeps falling (a trend): no recovery.
        let base = 100.0;
        let path = vec![(0u64, 100.0), (1_000_000_000, 99.0), (2_000_000_000, 98.0), (3_000_000_000, 97.0)];
        let r = measure_reversion(&path, 0, base, true);
        assert!(r.spike_bps > 200.0, "spike grows, got {}", r.spike_bps);
        assert!(r.revert_bps < 1.0, "no recovery, got {}", r.revert_bps);
        assert!(!r.full_revert);
    }

    #[test]
    fn fade_captures_bounce_at_delay() {
        // DOWN cascade -> we BUY. Enter at 800ms (px 99.0), price recovers to 100 (~101bps up).
        let path = vec![(0u64, 99.5), (800_000_000, 99.0), (1_600_000_000, 99.5), (2_400_000_000, 100.0)];
        // big target so we mark to market at the end of the hold.
        let cap = simulate_fade(&path, 0, true, 800_000_000, 1000.0, 30_000_000_000).unwrap();
        assert!(cap > 90.0, "captured the bounce, got {cap}");
        // target met early: exits at >= 50bps.
        let cap2 = simulate_fade(&path, 0, true, 800_000_000, 50.0, 30_000_000_000).unwrap();
        assert!(cap2 >= 50.0, "exits on target, got {cap2}");
    }

    #[test]
    fn fade_negative_when_trend_continues() {
        // DOWN cascade -> we BUY, but price keeps falling: captured is negative.
        let path = vec![(0u64, 100.0), (800_000_000, 99.5), (1_600_000_000, 99.0), (2_400_000_000, 98.5)];
        let cap = simulate_fade(&path, 0, true, 800_000_000, 50.0, 30_000_000_000).unwrap();
        assert!(cap < 0.0, "fading a trend loses, got {cap}");
    }

    #[test]
    fn fade_none_when_path_too_short() {
        let path = vec![(0u64, 100.0), (100_000_000, 100.0)];
        assert!(simulate_fade(&path, 0, true, 800_000_000, 10.0, 30_000_000_000).is_none());
    }

    #[test]
    fn exhaustion_fires_when_oi_and_price_stall() {
        // DOWN cascade sampled every 0.5s; stall window 1s, decel 0.5x.
        // Price extends to a low by 1.5s then goes FLAT (stalls); OI bleeds hard
        // over the first 1s then slows to a trickle => exhaustion should fire.
        let price = vec![
            (0u64, 100.0),
            (500_000_000, 99.5),
            (1_000_000_000, 99.2),
            (1_500_000_000, 99.1),
            (2_000_000_000, 99.1),
            (2_500_000_000, 99.1),
            (3_000_000_000, 99.2),
        ];
        let oi = vec![
            (0u64, 1000.0),
            (500_000_000, 980.0),
            (1_000_000_000, 965.0),
            (1_500_000_000, 960.0),
            (2_000_000_000, 959.0),
            (2_500_000_000, 958.0),
            (3_000_000_000, 958.0),
        ];
        let ex = find_exhaustion(&price, &oi, 0, true, 1_000_000_000, 0.5);
        assert_eq!(ex, Some(2_500_000_000), "exhaustion at 2.5s, got {ex:?}");
    }

    #[test]
    fn exhaustion_never_fires_on_a_trender() {
        // OI keeps bleeding AND price makes a new low every sample (a pure trend):
        // neither leg is ever satisfied => no exhaustion => the cascade is skipped.
        let price = vec![
            (0u64, 100.0),
            (500_000_000, 99.0),
            (1_000_000_000, 98.0),
            (1_500_000_000, 97.0),
            (2_000_000_000, 96.0),
            (2_500_000_000, 95.0),
            (3_000_000_000, 94.0),
        ];
        let oi = vec![
            (0u64, 1000.0),
            (500_000_000, 980.0),
            (1_000_000_000, 960.0),
            (1_500_000_000, 940.0),
            (2_000_000_000, 920.0),
            (2_500_000_000, 900.0),
            (3_000_000_000, 880.0),
        ];
        assert!(find_exhaustion(&price, &oi, 0, true, 1_000_000_000, 0.5).is_none());
    }

    #[test]
    fn magnitude_gate_filters_small_keeps_large() {
        // OFF (both thresholds 0) => every cascade passes (baseline byte-preserved).
        assert!(passes_magnitude(5.0, 0.1, 0.0, 0.0));
        assert!(passes_magnitude(200.0, 9.9, 0.0, 0.0));
        // min-spike 20: a 12bps realized move is filtered, a 35bps move passes.
        assert!(!passes_magnitude(12.0, 1.0, 20.0, 0.0));
        assert!(passes_magnitude(35.0, 1.0, 20.0, 0.0));
        // boundary is inclusive (>=).
        assert!(passes_magnitude(20.0, 1.0, 20.0, 0.0));
        // min-oidrop leg gates independently (AND of both legs).
        assert!(!passes_magnitude(50.0, 0.3, 20.0, 0.5));
        assert!(passes_magnitude(50.0, 0.6, 20.0, 0.5));
    }

    #[test]
    fn magnitude_gate_uses_realized_move_not_forward_peak() {
        // NO-LOOKAHEAD assertion. A cascade fires with only a SMALL realized
        // detection move (window-start -> fire), here 12bps, but its FORWARD path
        // overshoots to a big ~60bps peak afterwards. measure_reversion reports that
        // 60bps forward peak (lookahead). The gate must IGNORE the forward peak and
        // use ONLY the realized 12bps: at min-spike 20 it must FILTER this cascade,
        // even though the (future) spike is 60.
        let realized_move_bps = 12.0;
        let base = 100.0;
        // DOWN cascade path peaking ~60bps below baseline AFTER the fire.
        let fwd = vec![
            (0u64, 99.88),
            (1_000_000_000, 99.7),
            (2_000_000_000, 99.4),
            (3_000_000_000, 99.7),
        ];
        let rev = measure_reversion(&fwd, 0, base, true);
        assert!(rev.spike_bps > 50.0, "forward peak ~60bps (lookahead), got {}", rev.spike_bps);
        // The gate uses the realized move, NOT rev.spike_bps:
        assert!(
            !passes_magnitude(realized_move_bps, 1.0, 20.0, 0.0),
            "must filter on realized 12bps, not the 60bps forward peak"
        );
        // sanity: had we (wrongly) gated on the forward peak it would have passed.
        assert!(passes_magnitude(rev.spike_bps, 1.0, 20.0, 0.0));
    }

    #[test]
    fn confirm_revert_gate() {
        // DOWN cascade hit the BID; a revert is supported when bid depth RETURNS =>
        // imbalance RISES (imb_end > imb_start). A +0.30 shift passes a 0.10 thr.
        assert!(confirm_revert(-0.20, 0.10, true, 0.10), "bid returned => confirm");
        // imbalance keeps LEANING ask (depth stays pulled / thins) => no confirm.
        assert!(!confirm_revert(-0.20, -0.40, true, 0.10), "bid kept thinning => skip");
        // UP cascade hit the ASK; revert supported when ask depth returns => imb FALLS.
        assert!(confirm_revert(0.20, -0.10, false, 0.10), "ask returned => confirm");
        assert!(!confirm_revert(0.20, 0.40, false, 0.10), "ask kept thinning => skip");
        // exact boundary is inclusive (>=).
        assert!(confirm_revert(0.0, 0.10, true, 0.10));
    }

    #[test]
    fn maker_fills_through_level() {
        // DOWN cascade, reversion is UP; peak at t=1s. A maker SELL rests at baseline
        // 100.0 and fills iff an aggressive BUY prints >= 100 AFTER the peak.
        let trades = vec![
            (500_000_000u64, 99.0, true),   // before peak: ignored
            (1_500_000_000, 99.5, true),    // after peak but below level: no fill
            (2_000_000_000, 100.2, true),   // after peak, buy through 100 => FILL
        ];
        assert!(maker_fills_through(&trades, 1_000_000_000, 100.0, true), "tape traded up through baseline");
        // VACUUM: price re-quotes back but no aggressive buy prints through the level.
        let vacuum = vec![
            (1_500_000_000u64, 99.5, true),
            (2_000_000_000, 99.8, false),   // a SELL, wrong side
        ];
        assert!(!maker_fills_through(&vacuum, 1_000_000_000, 100.0, true), "no trade through => vacuum");
        // UP cascade, reversion DOWN: maker BUY at baseline fills on a SELL print <= level.
        let up = vec![(2_000_000_000u64, 99.7, false)];
        assert!(maker_fills_through(&up, 1_000_000_000, 100.0, false));
        // before-peak trades never fill (no-lookahead within the reversion leg).
        let early = vec![(900_000_000u64, 100.5, true)];
        assert!(!maker_fills_through(&early, 1_000_000_000, 100.0, true));
    }

    #[test]
    fn maker_in_path_fills_and_rides_bounce() {
        // DOWN cascade: rest a BID at touch (offset 0) at the already-dropped arm_px=100.
        // A forced SELL prints THROUGH at 99.5 (t=0.5s) -> FILL at the limit 100. Price
        // then bounces to 100.8 -> long from 100 -> ~+80bps captured (mark-to-market).
        let path = vec![(0u64, 100.0), (500_000_000, 99.5), (1_000_000_000, 100.0), (2_000_000_000, 100.8)];
        let trades = vec![(500_000_000u64, 99.5, false)]; // aggressive SELL through the bid
        let (fts, bps) = simulate_maker_fade(&path, &trades, 0, 100.0, true, 0.0, 1000.0, 30_000_000_000, None)
            .expect("forced sell fills the resting bid");
        assert_eq!(fts, 500_000_000);
        assert!(bps > 70.0, "rode the bounce long 100->100.8, got {bps}");
    }

    #[test]
    fn maker_in_path_trender_runs_over_is_a_loss() {
        // DOWN cascade: the bid fills as a forced sell prints through, then price KEEPS
        // DROPPING (a trender). A resting maker long bleeds = a REAL signed LOSS that
        // MUST be counted (no revert-conditioning). 100 -> 97.5 = ~-250bps.
        let path = vec![(0u64, 100.0), (500_000_000, 99.5), (1_000_000_000, 98.5), (2_000_000_000, 97.5)];
        let trades = vec![(500_000_000u64, 99.5, false)];
        let (_, bps) = simulate_maker_fade(&path, &trades, 0, 100.0, true, 0.0, 50.0, 30_000_000_000, None)
            .expect("fills, then runs over");
        assert!(bps < -100.0, "trender run-over must be a real loss, got {bps}");
    }

    #[test]
    fn maker_in_path_no_fill_no_trade() {
        // Bid rests 20bps below (level=99.8). The only sell prints at 99.99 (above the
        // level) -> never trades through -> NO FILL -> None (no trade, no cost).
        let path = vec![(0u64, 100.0), (500_000_000, 99.95), (1_000_000_000, 100.1)];
        let trades = vec![(500_000_000u64, 99.99, false)];
        assert!(simulate_maker_fade(&path, &trades, 0, 100.0, true, 20.0, 50.0, 30_000_000_000, None).is_none());
    }

    #[test]
    fn maker_hold_gate_cancels_unconfirmed_late_fill() {
        // The fill would print at 1.0s, AFTER the 800ms confirm read. Confirm FAILS ->
        // the still-resting order is CANCELLED at the read -> NO trade. With confirm OK
        // the same late fill is kept (the order stays resting and fills).
        let path = vec![(0u64, 100.0), (800_000_000, 99.5), (1_000_000_000, 99.4), (2_000_000_000, 99.0)];
        let trades = vec![(1_000_000_000u64, 99.4, false)];
        assert!(
            simulate_maker_fade(&path, &trades, 0, 100.0, true, 0.0, 50.0, 30_000_000_000, Some((800_000_000, false))).is_none(),
            "unconfirmed order cancelled before the late fill"
        );
        assert!(
            simulate_maker_fade(&path, &trades, 0, 100.0, true, 0.0, 50.0, 30_000_000_000, Some((800_000_000, true))).is_some(),
            "confirmed order stays resting and fills"
        );
    }

    #[test]
    fn maker_hold_gate_flattens_filled_trender_at_confirm() {
        // Fill at 0.5s (BEFORE the 800ms confirm). Confirm FAILS -> flatten at the read
        // (mark-to-market at 99.5 = -50bps) = cut the trender early, NOT held to the
        // deeper -100bps drop at 2s.
        let path = vec![(0u64, 100.0), (500_000_000, 99.7), (800_000_000, 99.5), (2_000_000_000, 99.0)];
        let trades = vec![(500_000_000u64, 99.7, false)];
        let (_, bps) = simulate_maker_fade(&path, &trades, 0, 100.0, true, 0.0, 1000.0, 30_000_000_000, Some((800_000_000, false)))
            .expect("filled before the cut");
        assert!((bps - (-50.0)).abs() < 5.0, "flattened at the confirm read ~ -50bps, got {bps}");
    }

    #[test]
    fn momentum_continuation_hits_target_is_a_win() {
        // DOWN cascade -> we SHORT. Price CONTINUES down (the overshoot): 100 -> 99.7
        // (~+30bps in our favour). target 25 / stop 12. First touch is the target.
        let path = vec![(0u64, 100.0), (500_000_000, 99.8), (1_000_000_000, 99.7)];
        let cap = simulate_momentum(&path, 0, true, 0, 25.0, 12.0, 30_000_000_000).unwrap();
        assert!(cap >= 25.0, "rode the continuation to target, got {cap}");
        // UP cascade -> we LONG. Price continues up 100 -> 100.3 (~+30bps). target hit.
        let up = vec![(0u64, 100.0), (500_000_000, 100.2), (1_000_000_000, 100.3)];
        let capu = simulate_momentum(&up, 0, false, 0, 25.0, 12.0, 30_000_000_000).unwrap();
        assert!(capu >= 25.0, "long continuation to target, got {capu}");
    }

    #[test]
    fn momentum_reverter_hits_stop_is_small_loss() {
        // DOWN cascade -> we SHORT, but the cascade SNAPS BACK (a reverter): price
        // reverts UP against our short to 100.12 (~-12bps). stop 12 -> stopped out at
        // the expected small loss (NOT the big target). The reverter IS counted.
        let path = vec![(0u64, 100.0), (500_000_000, 100.05), (1_000_000_000, 100.12)];
        let cap = simulate_momentum(&path, 0, true, 0, 25.0, 12.0, 30_000_000_000).unwrap();
        assert!(cap <= -12.0, "reverter stops out, got {cap}");
        assert!(cap > -30.0, "stop caps the loss small, got {cap}");
    }

    #[test]
    fn momentum_first_touch_stop_before_target_is_a_loss() {
        // FIRST-TOUCH ordering. DOWN cascade -> SHORT. Price first reverts UP to 100.13
        // (hits the -12 stop at 0.5s) and only LATER drops to 99.7 (which WOULD hit the
        // +25 target). Because the stop is touched FIRST in time, the trade is a LOSS -
        // the later target must NOT be credited (no lookahead past the exit).
        let stop_first = vec![(0u64, 100.0), (500_000_000, 100.13), (1_000_000_000, 99.7)];
        let cap = simulate_momentum(&stop_first, 0, true, 0, 25.0, 12.0, 30_000_000_000).unwrap();
        assert!(cap < 0.0, "stop touched first => loss, got {cap}");
        // Mirror: target touched FIRST (0.5s) then a later move that would breach the
        // stop -> the trade is a WIN, exited at the target before the reversal.
        let target_first = vec![(0u64, 100.0), (500_000_000, 99.7), (1_000_000_000, 100.13)];
        let capw = simulate_momentum(&target_first, 0, true, 0, 25.0, 12.0, 30_000_000_000).unwrap();
        assert!(capw >= 25.0, "target touched first => win, got {capw}");
    }

    #[test]
    fn momentum_none_when_path_too_short() {
        // entry delay is past the end of the path => no entry.
        let path = vec![(0u64, 100.0), (100_000_000, 100.0)];
        assert!(simulate_momentum(&path, 0, true, 800_000_000, 25.0, 12.0, 30_000_000_000).is_none());
    }

    #[test]
    fn momentum_trend_confirm_mirror_of_revert() {
        // DOWN cascade hit the BID. TREND holds when the bid does NOT return =>
        // imbalance does NOT rise toward bid (toward-hit shift <= 0). It stays leaning
        // ask (-0.2 -> -0.4) => confirm_trend TRUE. If the bid RETURNS (-0.2 -> +0.2,
        // toward-hit +0.4) the trend is NOT confirmed (that is the fade case).
        assert!(confirm_trend(-0.20, -0.40, true, 0.0), "bid stayed pulled => trend");
        assert!(!confirm_trend(-0.20, 0.20, true, 0.0), "bid returned => no trend");
        // UP cascade hit the ASK: trend holds when ask stays gone (imb does NOT fall).
        assert!(confirm_trend(0.20, 0.40, false, 0.0), "ask stayed pulled => trend");
        assert!(!confirm_trend(0.20, -0.20, false, 0.0), "ask returned => no trend");
        // it is the exact mirror of confirm_revert: a return that confirms a revert
        // must NOT confirm a trend, and vice-versa.
        assert!(confirm_revert(-0.20, 0.20, true, 0.10));
        assert!(!confirm_trend(-0.20, 0.20, true, 0.0));
    }
}