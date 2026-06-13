//! `gapscope` - a RESEARCH / ANALYSIS tool (NOT a strategy) that characterises
//! HOW the cross-venue basis gap CLOSES at the order-book level.
//!
//! Lagshot's basis-reversion edge is real but uncapturable as taker (latency)
//! and as maker (adverse selection). The PATH-mode tweak showed the fillable
//! flow is momentum/overshoot, not mean-reverting liquidity. The open question
//! the trader wants answered is at the BOOK level: when a dislocation closes,
//! does price move because of TRADES (someone lifting/hitting) or because of
//! RE-QUOTE (walls stacking / pulling with little trading, possibly spoofing)?
//! Do walls appear and ABSORB (real) or get PULLED (spoof-like)? Does dev
//! OVERSHOOT past zero (the maker getting run over)?
//!
//! It replays a real ETH day (reusing `load_window` + `forge_book::OrderBook` +
//! the `FairValueOracle` gap/baseline/dev math - sacred core untouched),
//! DETECTS dislocation events (|dev| >= --thr, same rolling-baseline dev as the
//! oracle), and for each records order-book behaviour over the CLOSE WINDOW
//! (until |dev| <= --exit or --horizon), emitting per-dislocation + aggregate
//! statistics. Deterministic, no-lookahead (only `<= now` state is read).
//!
//!   gapscope --root /root/chd/fresh/ticks --coin ETH --symbol ETH-USDT \
//!     --dates 2025-11-04,2025-11-20,2025-12-08 --hours all --thr 16 \
//!     [--exit 2 --horizon 5s --settle 1s --top 5 --dump rows.csv]

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, Price, Side, UnixNanos};
use forgelag::{load_window, FairValueConfig, FairValueOracle, FeedConfig, LagCtx, LagKind, Role};

// ----------------------------------------------------------------------------
// small parse helpers (mirroring hunt.rs conventions)
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// PURE, TESTED classification helpers (the heart of the study's definitions)
// ----------------------------------------------------------------------------

/// Top-N depth imbalance in [-1, 1]: `(bid - ask) / (bid + ask)`. Positive =
/// more bid depth (book leans up), negative = more ask depth (leans down).
#[must_use]
fn imbalance(bid_depth: f64, ask_depth: f64) -> f64 {
    let t = bid_depth + ask_depth;
    if t <= 0.0 {
        0.0
    } else {
        (bid_depth - ask_depth) / t
    }
}

/// CLOSE-MECHANISM classification. `total_toward` is the net microprice move
/// TOWARD fair value over the close window (price units or bps, sign matters);
/// `trade_toward` is the part of that move that happened within the trade
/// attribution window of an aggressive HL trade in the close direction.
/// Returns the trade-attributed FRACTION (0 when there was no net toward-move)
/// and whether the close is TRADE-DRIVEN (fraction >= `frac_thr`). A close where
/// price moved toward fair value with little contemporaneous trading is
/// RE-QUOTE-driven (returns false).
#[must_use]
fn classify_mechanism(trade_toward: f64, total_toward: f64, frac_thr: f64) -> (f64, bool) {
    if total_toward <= 0.0 {
        return (0.0, false);
    }
    let frac = (trade_toward / total_toward).clamp(0.0, 1.0);
    (frac, frac >= frac_thr)
}

/// WALL-RESOLUTION classification. A tracked wall (a top-of-book level whose
/// size was `>> median`) has shrunk from `peak` to `current`; `traded_during`
/// is the cumulative aggressive volume that printed AT that level's price while
/// the wall was alive. The size that DISAPPEARED is `peak - current`. If trades
/// explain at least `absorb_frac` of the disappearance it was ABSORBED (real
/// liquidity consumed by the tape); otherwise it was PULLED (cancelled without
/// trading = spoof-like).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WallOutcome {
    Absorbed,
    Pulled,
}

#[must_use]
fn classify_wall(peak: f64, current: f64, traded_during: f64, absorb_frac: f64) -> WallOutcome {
    let disappeared = (peak - current).max(0.0);
    if disappeared <= 0.0 {
        // nothing left (shouldn't happen at resolution); treat as pulled.
        return WallOutcome::Pulled;
    }
    if traded_during >= absorb_frac * disappeared {
        WallOutcome::Absorbed
    } else {
        WallOutcome::Pulled
    }
}

/// Median of a slice (sorts a copy). 0.0 for empty.
#[must_use]
fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// Size-weighted top-N microprice, copied from `FairValueOracle::micro` /
/// `BasisSignal::micro` so this tool and the validated dev agree exactly.
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

/// Top-N (bid_depth, ask_depth) in base units.
#[must_use]
fn top_depth(book: &OrderBook, top_n: usize) -> (f64, f64) {
    let bid: f64 = book.bids_iter().take(top_n).map(|(_, q)| q.to_f64()).sum();
    let ask: f64 = book.asks_iter().take(top_n).map(|(_, q)| q.to_f64()).sum();
    (bid, ask)
}
// ----------------------------------------------------------------------------
// configuration + per-dislocation record
// ----------------------------------------------------------------------------

#[derive(Clone)]
struct Cfg {
    root: PathBuf,
    coin: String,
    symbol: String,
    hours: Vec<String>,
    thr_bps: f64,
    exit_bps: f64,
    horizon_ns: u64,
    settle_ns: u64,
    top_n: usize,
    window: usize,
    sample_ns: u64,
    wall_mult: f64,
    absorb_frac: f64,
    attrib_ns: u64,
    frac_thr: f64,
}

/// One detected dislocation and the book behaviour over its close window.
#[derive(Clone, Default)]
struct Disloc {
    day: String,
    start_ts: u64,
    start_dev: f64,
    dir_down: bool,
    closed: bool,
    ttc_ns: u64,
    // mechanism
    total_toward: f64,
    trade_toward: f64,
    micro_move_bps: f64,
    trade_frac: f64,
    trade_driven: bool,
    close_vol: f64,
    against_vol: f64,
    // walls
    w_app: u32,
    w_app_bid: u32,
    w_app_ask: u32,
    w_abs: u32,
    w_pull: u32,
    w_stand: u32,
    // imbalance
    imb_start: f64,
    imb_end: f64,
    imb_toward_close: f64,
    // overshoot / extent
    overshoot_bps: f64,
    did_overshoot: bool,
    peak_abs_dev: f64,
}

/// A top-of-book level large enough to be a "wall", tracked from appearance
/// until it shrinks back (resolved as absorbed or pulled) or the window ends.
#[derive(Clone, Copy)]
struct ActiveWall {
    side: Side,
    price_raw: i64,
    peak: f64,
    traded_baseline: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Armed,
    Tracking,
    Settling,
    WaitRearm,
}

/// Replays one day and returns its detected dislocations. Deterministic and
/// no-lookahead: the book/ref/oracle are folded forward event by event and only
/// `<= now` state is ever read.
fn scan_day(cfg: &Cfg, day: &str, evs: &[forgelag::LagEvent]) -> Vec<Disloc> {
    let mut book = OrderBook::with_max_levels(64);
    let mut oracle = FairValueOracle::new(FairValueConfig {
        top_n: cfg.top_n,
        window: cfg.window,
        sample_ns: cfg.sample_ns,
        staleness_ns: forgelag::DEFAULT_STALENESS_NS,
    });
    let mut ref_last: Vec<f64> = Vec::new();
    let mut ref_px = 0.0f64;

    let mut out: Vec<Disloc> = Vec::new();
    let mut state = State::Armed;
    let mut cur = Disloc::default();
    let mut micro_prev: Option<f64> = None;
    let mut recent_close_trade_until: u64 = 0;
    let mut min_dev = f64::INFINITY;
    let mut max_dev = f64::NEG_INFINITY;
    let mut settle_until: u64 = 0;
    let mut start_ref: f64 = 0.0;
    // wall bookkeeping (reset per dislocation)
    let mut trade_at_price: HashMap<i64, f64> = HashMap::new();
    let mut walls: Vec<ActiveWall> = Vec::new();

    for ev in evs {
        let now = ev.local_ts;
        // ---- fold the event into book / reference (sacred ordering) ----
        let mut is_book_update = false;
        let mut hl_trade: Option<(Side, i64, f64)> = None;
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
                        is_book_update = true;
                    }
                }
            }
            (Role::Exec, LagKind::Trade) => {
                if let Some(aggr) = ev.side {
                    hl_trade = Some((aggr, ev.price.raw(), ev.qty.to_f64()));
                }
            }
            (Role::Reference, LagKind::Trade) => {
                let i = ev.src as usize;
                if ref_last.len() <= i {
                    ref_last.resize(i + 1, 0.0);
                }
                ref_last[i] = ev.price.to_f64();
                let (sum, cnt) = ref_last.iter().filter(|&&x| x > 0.0).fold((0.0, 0u32), |(s, c), &x| (s + x, c + 1));
                ref_px = if cnt > 0 { sum / f64::from(cnt) } else { 0.0 };
            }
            _ => {}
        }

        // ---- oracle observe (maintains the validated rolling baseline) ----
        let ctx = LagCtx { now, exec_book: &book, ref_px, funding: 0.0, oi: 0.0, lead_px: 0.0, position_qty: 0 };
        oracle.observe(&ctx);

        // ---- event-resolution instantaneous dev using the oracle baseline ----
        // (gap minus the same rolling baseline; finer timing than the 500ms
        //  sample cadence while keeping the validated baseline math.)
        let m_now = micro(&book, cfg.top_n);
        let dev = if oracle.ready() && !oracle.is_stale(now) {
            match (m_now, ref_px > 0.0) {
                (Some(m), true) => Some((m - ref_px) / ref_px * 10_000.0 - oracle.baseline_bps()),
                _ => None,
            }
        } else {
            None
        };

        // accumulate per-price HL trade volume while a dislocation is open.
        if matches!(state, State::Tracking) {
            if let Some((aggr, praw, q)) = hl_trade {
                *trade_at_price.entry(praw).or_insert(0.0) += q;
                // close-direction aggressor? rich(dir_down) closes via SELLS (Ask aggressor);
                // cheap closes via BUYS (Bid aggressor).
                let close_aggr = if cur.dir_down { Side::Ask } else { Side::Bid };
                if aggr == close_aggr {
                    cur.close_vol += q;
                    recent_close_trade_until = now + cfg.attrib_ns;
                } else {
                    cur.against_vol += q;
                }
            }
        }

        // ---- mechanism + microprice attribution (during TRACKING only) ----
        if matches!(state, State::Tracking) {
            if let (Some(mp), Some(mn)) = (micro_prev, m_now) {
                let d = mn - mp;
                let toward = if cur.dir_down { -d } else { d };
                cur.total_toward += toward;
                if now <= recent_close_trade_until {
                    cur.trade_toward += toward;
                }
            }
        }
        if m_now.is_some() {
            micro_prev = m_now;
        }

        // ---- wall dynamics (during TRACKING, on book updates) ----
        if matches!(state, State::Tracking) && is_book_update {
            // median top size from current top-N both sides.
            let mut sizes: Vec<f64> = Vec::with_capacity(cfg.top_n * 2);
            sizes.extend(book.bids_iter().take(cfg.top_n).map(|(_, q)| q.to_f64()));
            sizes.extend(book.asks_iter().take(cfg.top_n).map(|(_, q)| q.to_f64()));
            let med = median(&sizes);
            let wall_thr = med * cfg.wall_mult;
            if wall_thr > 0.0 {
                // detect NEW walls in the current top-N.
                for (side, it) in [(Side::Bid, book.bids_iter().take(cfg.top_n).collect::<Vec<_>>()),
                                   (Side::Ask, book.asks_iter().take(cfg.top_n).collect::<Vec<_>>())] {
                    for (p, q) in it {
                        let sz = q.to_f64();
                        if sz >= wall_thr {
                            let praw = p.raw();
                            if !walls.iter().any(|w| w.side == side && w.price_raw == praw) {
                                walls.push(ActiveWall {
                                    side,
                                    price_raw: praw,
                                    peak: sz,
                                    traded_baseline: trade_at_price.get(&praw).copied().unwrap_or(0.0),
                                });
                                cur.w_app += 1;
                                if side == Side::Bid { cur.w_app_bid += 1; } else { cur.w_app_ask += 1; }
                            }
                        }
                    }
                }
            }
            // update peaks + resolve shrunk walls.
            let absorb_frac = cfg.absorb_frac;
            walls.retain_mut(|w| {
                let cur_sz = book.qty_at(w.side, Price::from_raw(w.price_raw)).map_or(0.0, |q| q.to_f64());
                if cur_sz > w.peak {
                    w.peak = cur_sz;
                }
                if cur_sz <= 0.4 * w.peak {
                    let traded = trade_at_price.get(&w.price_raw).copied().unwrap_or(0.0) - w.traded_baseline;
                    match classify_wall(w.peak, cur_sz, traded.max(0.0), absorb_frac) {
                        WallOutcome::Absorbed => cur.w_abs += 1,
                        WallOutcome::Pulled => cur.w_pull += 1,
                    }
                    false
                } else {
                    true
                }
            });
        }
        // ---- dev-driven state machine (event-resolution timing) ----
        if let Some(dv) = dev {
            match state {
                State::Armed => {
                    if dv.abs() >= cfg.thr_bps {
                        cur = Disloc { day: day.to_string(), start_ts: now, start_dev: dv, dir_down: dv > 0.0, ..Disloc::default() };
                        let (bd, ad) = top_depth(&book, cfg.top_n);
                        cur.imb_start = imbalance(bd, ad);
                        start_ref = ref_px;
                        min_dev = dv;
                        max_dev = dv;
                        micro_prev = m_now;
                        recent_close_trade_until = 0;
                        trade_at_price.clear();
                        walls.clear();
                        state = State::Tracking;
                    }
                }
                State::Tracking => {
                    if dv < min_dev { min_dev = dv; }
                    if dv > max_dev { max_dev = dv; }
                    if dv.abs() <= cfg.exit_bps {
                        cur.closed = true;
                        cur.ttc_ns = now.saturating_sub(cur.start_ts);
                        settle_until = now + cfg.settle_ns;
                        state = State::Settling;
                    } else if now.saturating_sub(cur.start_ts) > cfg.horizon_ns {
                        finalize(&mut cur, &book, cfg, start_ref, min_dev, max_dev, &mut walls);
                        out.push(cur.clone());
                        trade_at_price.clear();
                        state = State::WaitRearm;
                    }
                }
                State::Settling => {
                    if dv < min_dev { min_dev = dv; }
                    if dv > max_dev { max_dev = dv; }
                    if now >= settle_until {
                        finalize(&mut cur, &book, cfg, start_ref, min_dev, max_dev, &mut walls);
                        out.push(cur.clone());
                        trade_at_price.clear();
                        state = State::WaitRearm;
                    }
                }
                State::WaitRearm => {
                    if dv.abs() < cfg.thr_bps {
                        state = State::Armed;
                    }
                }
            }
        }
    }

    // a dislocation still open at end-of-day is dropped (incomplete window).
    out
}

/// Finalise a dislocation: compute the mechanism classification, microprice
/// move (bps), end imbalance + toward-close shift, overshoot, peak |dev|, and
/// count any walls still standing. Clears the active-wall list.
fn finalize(
    cur: &mut Disloc,
    book: &OrderBook,
    cfg: &Cfg,
    start_ref: f64,
    min_dev: f64,
    max_dev: f64,
    walls: &mut Vec<ActiveWall>,
) {
    cur.micro_move_bps = if start_ref > 0.0 { cur.total_toward / start_ref * 10_000.0 } else { 0.0 };
    let (frac, td) = classify_mechanism(cur.trade_toward, cur.total_toward, cfg.frac_thr);
    cur.trade_frac = frac;
    cur.trade_driven = td;
    let (bd, ad) = top_depth(book, cfg.top_n);
    cur.imb_end = imbalance(bd, ad);
    cur.imb_toward_close = if cur.dir_down { -(cur.imb_end - cur.imb_start) } else { cur.imb_end - cur.imb_start };
    cur.overshoot_bps = if cur.dir_down { (-min_dev).max(0.0) } else { max_dev.max(0.0) };
    cur.did_overshoot = cur.overshoot_bps > cfg.exit_bps;
    cur.peak_abs_dev = max_dev.abs().max(min_dev.abs());
    cur.w_stand = walls.len() as u32;
    walls.clear();
}

// ----------------------------------------------------------------------------
// aggregate reporting
// ----------------------------------------------------------------------------

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

fn report(label: &str, ds: &[Disloc], cfg: &Cfg) {
    println!("\n==================== {label} ====================");
    let n = ds.len();
    println!("dislocations (|dev|>={}bps)   {n}", cfg.thr_bps);
    if n == 0 {
        println!("  (none detected - quiet regime at this threshold)");
        return;
    }
    let closed: Vec<&Disloc> = ds.iter().filter(|d| d.closed).collect();
    let nc = closed.len();
    println!("closed within {}                {nc}/{n} = {:.1}%", fmt_ns(cfg.horizon_ns), nc as f64 / n as f64 * 100.0);

    // time-to-close (closed only)
    let ttc_ms: Vec<f64> = closed.iter().map(|d| d.ttc_ns as f64 / 1e6).collect();
    if !ttc_ms.is_empty() {
        println!("time-to-close ms              median {:.0}  mean {:.0}", median(&ttc_ms), mean(&ttc_ms));
    }

    // mechanism (closed only - it characterises the close)
    if nc > 0 {
        let td = closed.iter().filter(|d| d.trade_driven).count();
        let fr: Vec<f64> = closed.iter().map(|d| d.trade_frac).collect();
        println!(
            "CLOSE MECHANISM               TRADE-driven {}/{nc} = {:.1}%   REQUOTE-driven {:.1}%   (mean trade-frac {:.2})",
            td,
            td as f64 / nc as f64 * 100.0,
            (nc - td) as f64 / nc as f64 * 100.0,
            mean(&fr)
        );
        let cv: Vec<f64> = closed.iter().map(|d| d.close_vol).collect();
        let av: Vec<f64> = closed.iter().map(|d| d.against_vol).collect();
        println!("  close-dir aggr vol (base)   mean {:.3}   against-dir {:.3}", mean(&cv), mean(&av));
        let mm: Vec<f64> = closed.iter().map(|d| d.micro_move_bps).collect();
        println!("  net microprice move toward  mean {:.2}bps", mean(&mm));
    }

    // walls (all dislocations)
    let app: u32 = ds.iter().map(|d| d.w_app).sum();
    let appb: u32 = ds.iter().map(|d| d.w_app_bid).sum();
    let appa: u32 = ds.iter().map(|d| d.w_app_ask).sum();
    let abs: u32 = ds.iter().map(|d| d.w_abs).sum();
    let pull: u32 = ds.iter().map(|d| d.w_pull).sum();
    let stand: u32 = ds.iter().map(|d| d.w_stand).sum();
    let resolved = abs + pull;
    let with_pull = ds.iter().filter(|d| d.w_pull > 0).count();
    let with_abs = ds.iter().filter(|d| d.w_abs > 0).count();
    println!("WALLS  appeared {app} (bid {appb}/ask {appa})  absorbed {abs}  pulled {pull}  standing {stand}");
    if resolved > 0 {
        println!(
            "  of RESOLVED walls: absorbed {:.1}%  pulled(spoof-like) {:.1}%",
            abs as f64 / resolved as f64 * 100.0,
            pull as f64 / resolved as f64 * 100.0
        );
    }
    println!(
        "  dislocations with >=1 pull {:.1}%   with >=1 absorb {:.1}%",
        with_pull as f64 / n as f64 * 100.0,
        with_abs as f64 / n as f64 * 100.0
    );

    // depth imbalance
    let imbsh: Vec<f64> = ds.iter().map(|d| d.imb_toward_close).collect();
    let imbs: Vec<f64> = ds.iter().map(|d| d.imb_start).collect();
    let imbe: Vec<f64> = ds.iter().map(|d| d.imb_end).collect();
    println!(
        "DEPTH IMBALANCE               start {:.3}  end {:.3}  mean toward-close shift {:+.3}",
        mean(&imbs), mean(&imbe), mean(&imbsh)
    );

    // overshoot
    let ov = ds.iter().filter(|d| d.did_overshoot).count();
    let ovb: Vec<f64> = ds.iter().filter(|d| d.did_overshoot).map(|d| d.overshoot_bps).collect();
    println!(
        "OVERSHOOT (dev flips sign)    {}/{n} = {:.1}%   median overshoot {:.1}bps",
        ov,
        ov as f64 / n as f64 * 100.0,
        if ovb.is_empty() { 0.0 } else { median(&ovb) }
    );
    let nclosed_peak: Vec<f64> = ds.iter().filter(|d| !d.closed).map(|d| d.peak_abs_dev).collect();
    if !nclosed_peak.is_empty() {
        println!("  did-not-close peak |dev|    mean {:.1}bps (n={})", mean(&nclosed_peak), nclosed_peak.len());
    }
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 { format!("{}s", ns / 1_000_000_000) } else { format!("{}ms", ns / 1_000_000) }
}
// ----------------------------------------------------------------------------
// main / CLI
// ----------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let mut cfg = Cfg {
        root: PathBuf::from("/root/chd/fresh/ticks"),
        coin: "ETH".to_string(),
        symbol: "ETH-USDT".to_string(),
        hours: (0..24).map(|h| format!("{h:02}")).collect(),
        thr_bps: 16.0,
        exit_bps: 2.0,
        horizon_ns: 5_000_000_000,
        settle_ns: 1_000_000_000,
        top_n: 5,
        window: 500,
        sample_ns: 500_000_000,
        wall_mult: 5.0,
        absorb_frac: 0.5,
        attrib_ns: 100_000_000,
        frac_thr: 0.5,
    };
    let mut dates: Vec<String> = Vec::new();
    let mut dump: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--root" => cfg.root = PathBuf::from(val()?),
            "--coin" => cfg.coin = val()?,
            "--symbol" => cfg.symbol = val()?,
            "--dates" => dates = val()?.split(',').map(|s| s.trim().to_string()).collect(),
            "--hours" => {
                let h = val()?;
                cfg.hours = if h == "all" {
                    (0..24).map(|x| format!("{x:02}")).collect()
                } else {
                    h.split(',').map(|s| s.trim().to_string()).collect()
                };
            }
            "--thr" => cfg.thr_bps = val()?.parse().map_err(|e| format!("thr: {e}"))?,
            "--exit" => cfg.exit_bps = val()?.parse().map_err(|e| format!("exit: {e}"))?,
            "--horizon" => cfg.horizon_ns = parse_dur(&val()?)?,
            "--settle" => cfg.settle_ns = parse_dur(&val()?)?,
            "--top" => cfg.top_n = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--window" => cfg.window = val()?.parse().map_err(|e| format!("window: {e}"))?,
            "--sample" => cfg.sample_ns = parse_dur(&val()?)?,
            "--wall-mult" => cfg.wall_mult = val()?.parse().map_err(|e| format!("wall-mult: {e}"))?,
            "--absorb-frac" => cfg.absorb_frac = val()?.parse().map_err(|e| format!("absorb-frac: {e}"))?,
            "--attrib" => cfg.attrib_ns = parse_dur(&val()?)?,
            "--frac-thr" => cfg.frac_thr = val()?.parse().map_err(|e| format!("frac-thr: {e}"))?,
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

    println!("gapscope - basis gap-close order-book study (analysis only, no trading)");
    println!("coin {}  ref {}  dates {}", cfg.coin, cfg.symbol, dates.join(","));
    println!(
        "thr {}bps  exit {}bps  horizon {}  settle {}  top {}  baseline win {}/{}",
        cfg.thr_bps, cfg.exit_bps, fmt_ns(cfg.horizon_ns), fmt_ns(cfg.settle_ns), cfg.top_n, cfg.window, fmt_ns(cfg.sample_ns)
    );
    println!(
        "wall>= {}x median-top  absorb if traded>= {} of vanished  trade-attrib window {}  trade-frac thr {}",
        cfg.wall_mult, cfg.absorb_frac, fmt_ns(cfg.attrib_ns), cfg.frac_thr
    );

    let mut pooled: Vec<Disloc> = Vec::new();
    for d in &dates {
        let fc = FeedConfig {
            root: cfg.root.clone(),
            coin: cfg.coin.clone(),
            ref_symbols: cfg.symbol.split(',').map(|s| s.trim().to_string()).collect(),
            lead_symbols: Vec::new(),
            date: d.clone(),
            hours: cfg.hours.clone(),
            exec_latency_ns: 0,
            ref_latency_ns: 0,
        };
        let evs = match load_window(&fc) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  skip {d}: {e}");
                continue;
            }
        };
        if evs.is_empty() {
            eprintln!("  skip {d}: empty");
            continue;
        }
        eprintln!("  {d}: {} events", evs.len());
        let ds = scan_day(&cfg, d, &evs);
        report(&format!("DAY {d}"), &ds, &cfg);
        pooled.extend(ds);
    }

    report("POOLED (all days)", &pooled, &cfg);

    if let Some(path) = &dump {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| format!("dump: {e}"))?;
        writeln!(f, "day,start_ts,start_dev_bps,dir_down,closed,ttc_ms,trade_frac,trade_driven,micro_move_bps,total_toward,trade_toward,close_vol,against_vol,w_app,w_app_bid,w_app_ask,w_abs,w_pull,w_stand,imb_start,imb_end,imb_toward_close,overshoot_bps,did_overshoot,peak_abs_dev").map_err(|e| format!("dump: {e}"))?;
        for d in &pooled {
            writeln!(
                f,
                "{},{},{:.3},{},{},{:.1},{:.4},{},{:.3},{:.6},{:.6},{:.4},{:.4},{},{},{},{},{},{},{:.4},{:.4},{:.4},{:.2},{},{:.2}",
                d.day, d.start_ts, d.start_dev, d.dir_down, d.closed,
                d.ttc_ns as f64 / 1e6, d.trade_frac, d.trade_driven, d.micro_move_bps,
                d.total_toward, d.trade_toward, d.close_vol, d.against_vol,
                d.w_app, d.w_app_bid, d.w_app_ask, d.w_abs, d.w_pull, d.w_stand,
                d.imb_start, d.imb_end, d.imb_toward_close, d.overshoot_bps, d.did_overshoot, d.peak_abs_dev
            )
            .map_err(|e| format!("dump: {e}"))?;
        }
        eprintln!("dumped {} dislocation rows to {path}", pooled.len());
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
    fn imbalance_basic() {
        assert!((imbalance(0.0, 0.0)).abs() < 1e-12, "no depth => 0");
        assert!((imbalance(3.0, 1.0) - 0.5).abs() < 1e-12, "more bids => positive");
        assert!((imbalance(1.0, 3.0) + 0.5).abs() < 1e-12, "more asks => negative");
    }

    #[test]
    fn mechanism_trade_vs_requote() {
        // most of the toward-move happened near a trade => trade-driven.
        let (frac, td) = classify_mechanism(0.8, 1.0, 0.5);
        assert!((frac - 0.8).abs() < 1e-12);
        assert!(td, "0.8 >= 0.5 => trade-driven");
        // price moved toward fv with almost no trades => requote-driven.
        let (frac, td) = classify_mechanism(0.1, 1.0, 0.5);
        assert!((frac - 0.1).abs() < 1e-12);
        assert!(!td, "0.1 < 0.5 => requote-driven");
        // no net toward-move => not trade-driven, frac 0.
        let (frac, td) = classify_mechanism(0.0, 0.0, 0.5);
        assert!(frac == 0.0 && !td);
    }

    #[test]
    fn wall_absorbed_when_trades_explain_disappearance() {
        // peak 10 -> current 0, traded 8 of the 10 that vanished, frac 0.5 => absorbed.
        assert_eq!(classify_wall(10.0, 0.0, 8.0, 0.5), WallOutcome::Absorbed);
        // peak 10 -> current 0, only 1 traded => cancelled = pulled (spoof-like).
        assert_eq!(classify_wall(10.0, 0.0, 1.0, 0.5), WallOutcome::Pulled);
        // boundary: traded exactly == absorb_frac * disappeared => absorbed.
        assert_eq!(classify_wall(10.0, 0.0, 5.0, 0.5), WallOutcome::Absorbed);
    }

    #[test]
    fn median_handles_even_and_odd() {
        assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-12);
        assert!((median(&[4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < 1e-12);
        assert!(median(&[]) == 0.0);
    }
}