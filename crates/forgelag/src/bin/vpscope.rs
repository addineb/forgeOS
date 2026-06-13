//! `vpscope` - VOLUME-PROFILE low-volume-node (LVN) MEAN-REVERSION study (analysis
//! only, NOT a strategy). Builds a ROLLING, NO-LOOKAHEAD volume profile from HL
//! aggressive trades over a lookback L, marks the POC / value area / low-volume
//! nodes, and tests the auction-theory thesis: when price reaches a LOW-VOLUME
//! node OUTSIDE the value area it MEAN-REVERTS toward value (the POC).
//!
//! Sacred core untouched; reuses load_window + forge_book::OrderBook. Deterministic,
//! event-driven; first-touch trades with run-overs INCLUDED and realistic fees.
//! STEP 1 of the plan: test the LOCATION edge ALONE (no exhaustion conditioning).
//!
//! Example:
//!   vpscope --coin ETH --dates 2025-11-04,... --hours all
//!     --lookbacks 2h,4h --nbins 50 --lvn-frac 0.15,0.20 --va-frac 0.70
//!     --refresh 60s --sample 5s --forward 2h --cooldown 30m
//!     --class-thr 40 --stop 40 --min-dist 15 --min-trades 200

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitCode;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, UnixNanos};
use forgelag::{load_window, FeedConfig, LagEvent, LagKind, Role};

// ----------------------------------------------------------------------------
// small parse / math helpers (mirror sweepscope/oiscope conventions)
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

fn parse_f64_list(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|x| x.trim().parse::<f64>().map_err(|e| format!("bad number `{x}`: {e}"))).collect()
}

fn parse_dur_list(s: &str) -> Result<Vec<u64>, String> {
    s.split(',').map(|x| parse_dur(x.trim())).collect()
}

fn parse_usize_list(s: &str) -> Result<Vec<usize>, String> {
    s.split(',').map(|x| x.trim().parse::<usize>().map_err(|e| format!("bad int `{x}`: {e}"))).collect()
}

/// Size-weighted top-N microprice (copied from sweepscope/oiscope).
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

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

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

fn fmt_ns(ns: u64) -> String {
    if ns >= 3_600_000_000_000 {
        format!("{:.0}h", ns as f64 / 3.6e12)
    } else if ns >= 60_000_000_000 {
        format!("{:.0}m", ns as f64 / 60e9)
    } else if ns >= 1_000_000_000 {
        format!("{:.0}s", ns as f64 / 1e9)
    } else {
        format!("{}ms", ns / 1_000_000)
    }
}

/// Honest first-touch TAKER trade. Enter at first sample at/after `entry_ts`;
/// `long` = buy side. Exit by the FIRST of: `target_bps` favourable, `stop_bps`
/// adverse, or `hold_ns`. Returns signed bps (run-overs INCLUDED). Pure.
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
// ----------------------------------------------------------------------------
// VOLUME PROFILE (pure, tested): bin aggressive volume by price over a window
// ----------------------------------------------------------------------------

/// A volume profile over a price range, binned. Built from `(ts, price, qty)`
/// aggressive trades that the caller has already restricted to [now-L, now]
/// (no-lookahead is the caller's responsibility). Pure.
struct Profile {
    pmin: f64,
    bin_w: f64,
    nbins: usize,
    vol: Vec<f64>,
    max_vol: f64,
    poc_px: f64,
    va_lo_px: f64,
    va_hi_px: f64,
}

impl Profile {
    /// Build from trades. `nbins` price bins across [pmin,pmax]; value area grown
    /// from the POC outward (Market-Profile style) until `va_frac` of total volume
    /// is covered. Returns None if too few trades or a degenerate range.
    #[must_use]
    fn build(trades: &[(u64, f64, f64)], nbins: usize, va_frac: f64, min_trades: usize) -> Option<Self> {
        if nbins < 3 || trades.len() < min_trades {
            return None;
        }
        let mut pmin = f64::INFINITY;
        let mut pmax = f64::NEG_INFINITY;
        for &(_, px, _) in trades {
            if px > 0.0 {
                if px < pmin { pmin = px; }
                if px > pmax { pmax = px; }
            }
        }
        if pmax <= pmin || !pmin.is_finite() {
            return None;
        }
        let bin_w = (pmax - pmin) / nbins as f64;
        if bin_w <= 0.0 {
            return None;
        }
        let mut vol = vec![0.0f64; nbins];
        for &(_, px, q) in trades {
            if px <= 0.0 || !q.is_finite() || q <= 0.0 {
                continue;
            }
            let mut idx = ((px - pmin) / bin_w) as usize;
            if idx >= nbins { idx = nbins - 1; }
            vol[idx] += q;
        }
        let total: f64 = vol.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut poc_idx = 0usize;
        let mut max_vol = 0.0f64;
        for (i, &v) in vol.iter().enumerate() {
            if v > max_vol {
                max_vol = v;
                poc_idx = i;
            }
        }
        // value area: expand from POC toward the heavier neighbour until covered.
        let (mut lo, mut hi) = (poc_idx, poc_idx);
        let mut covered = vol[poc_idx];
        while covered < va_frac * total && (lo > 0 || hi < nbins - 1) {
            let below = if lo > 0 { vol[lo - 1] } else { -1.0 };
            let above = if hi < nbins - 1 { vol[hi + 1] } else { -1.0 };
            if above >= below {
                hi += 1;
                covered += vol[hi];
            } else {
                lo -= 1;
                covered += vol[lo];
            }
        }
        Some(Self {
            pmin,
            bin_w,
            nbins,
            vol,
            max_vol,
            poc_px: pmin + (poc_idx as f64 + 0.5) * bin_w,
            va_lo_px: pmin + lo as f64 * bin_w,
            va_hi_px: pmin + (hi as f64 + 1.0) * bin_w,
        })
    }

    /// Volume of the bin containing `px` (0 if outside the profile range).
    #[must_use]
    fn vol_at(&self, px: f64) -> f64 {
        if px < self.pmin {
            return 0.0;
        }
        let idx = ((px - self.pmin) / self.bin_w) as usize;
        if idx >= self.nbins { 0.0 } else { self.vol[idx] }
    }

    /// LOW-VOLUME NODE test: the bin holding `px` is thin (<= `lvn_frac` * POC vol).
    #[must_use]
    fn is_lvn(&self, px: f64, lvn_frac: f64) -> bool {
        self.max_vol > 0.0 && self.vol_at(px) <= lvn_frac * self.max_vol
    }

    /// price is OUTSIDE the value area.
    #[must_use]
    fn outside_va(&self, px: f64) -> bool {
        px < self.va_lo_px || px > self.va_hi_px
    }

    /// fraction of POC volume at `px` (the "thinness", for reporting).
    #[must_use]
    fn vol_frac(&self, px: f64) -> f64 {
        if self.max_vol > 0.0 { self.vol_at(px) / self.max_vol } else { 1.0 }
    }
}

/// Classify the forward path of an LVN detection (no-lookahead, first-touch).
/// `long` = reversion side (toward the POC). REVERSION (+1) if price reaches the
/// POC (favourable >= `dist_to_poc`) before extending `class_thr_bps` AWAY from
/// the POC; CONTINUATION (-1) if it extends away first; 0 = neither in horizon.
/// Also returns (revert_mag, cont_mag) = max favourable / adverse excursion bps.
#[must_use]
fn classify_vp(path: &[(u64, f64)], entry_ts: u64, dist_to_poc: f64, long: bool, class_thr_bps: f64) -> (i8, f64, f64) {
    let Some(start) = path.iter().position(|&(ts, _)| ts >= entry_ts) else {
        return (0, 0.0, 0.0);
    };
    let entry_px = path[start].1;
    if entry_px <= 0.0 {
        return (0, 0.0, 0.0);
    }
    let s = if long { 1.0 } else { -1.0 };
    let (mut max_fav, mut max_adv, mut class) = (0.0f64, 0.0f64, 0i8);
    for &(_, px) in &path[start..] {
        let signed = s * (px - entry_px) / entry_px * 10_000.0;
        if signed > max_fav { max_fav = signed; }
        if -signed > max_adv { max_adv = -signed; }
        if class == 0 {
            if dist_to_poc > 0.0 && signed >= dist_to_poc {
                class = 1; // reached value = reversion
            } else if class_thr_bps > 0.0 && -signed >= class_thr_bps {
                class = -1; // extended away = continuation
            }
        }
    }
    (class, max_fav, max_adv)
}
// ----------------------------------------------------------------------------
// config + one detected LVN reversion + per-day replay
// ----------------------------------------------------------------------------

struct Cfg {
    root: PathBuf,
    coin: String,
    hours: Vec<String>,
    refresh_ns: u64,
    sample_ns: u64,
    forward_ns: u64,
    hold_ns: u64,
    cooldown_ns: u64,
    top_n: usize,
    va_frac: f64,
    class_thr_bps: f64,
    stop_bps: f64,
    min_dist_bps: f64,
    min_trades: usize,
    fee_bps: f64,
}

/// One detected LVN-reversion candidate + its forward characterisation (scalars).
#[derive(Clone)]
#[allow(dead_code)] // several fields are kept for clarity / future dump
struct Det {
    day: String,
    l_ns: u64,
    nbins: usize,
    lvn_frac: f64,
    fire_ts: u64,
    long: bool,
    entry_px: f64,
    poc_px: f64,
    dist_to_poc_bps: f64,
    lvn_vol_frac: f64,
    class: i8,
    revert_mag: f64,
    cont_mag: f64,
    revert_take: Option<f64>,
}

#[derive(Default)]
struct PendingV {
    fire_ts: u64,
    long: bool,
    entry_px: f64,
    poc_px: f64,
    dist_to_poc_bps: f64,
    lvn_vol_frac: f64,
    end_ts: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Armed,
    Collecting,
}

/// Replay one day for ONE (lookback L, nbins, lvn_frac) triple. Deterministic +
/// no-lookahead: the profile is built ONLY from aggressive trades in [now-L, now]
/// (the current detection tick's price is read from the book state folded so far);
/// the forward path is collected AFTER the detection. The profile is rebuilt every
/// `refresh_ns` and detection runs at `sample_ns` cadence against the last profile.
fn scan_day(cfg: &Cfg, day: &str, evs: &[LagEvent], l_ns: u64, nbins: usize, lvn_frac: f64) -> Vec<Det> {
    let mut book = OrderBook::with_max_levels(64);
    let mut micro_now: Option<f64> = None;
    let mut vptrades: VecDeque<(u64, f64, f64)> = VecDeque::new();
    let mut profile: Option<Profile> = None;
    let mut last_refresh = 0u64;
    let mut last_sample = 0u64;
    let mut state = St::Armed;
    let mut blocked_until = 0u64;
    let mut cur = PendingV::default();
    let mut cur_path: Vec<(u64, f64)> = Vec::new();
    let mut out: Vec<Det> = Vec::new();

    for ev in evs {
        let now = ev.local_ts;
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
            (Role::Exec, LagKind::Trade) if ev.side.is_some() => {
                let q = ev.qty.to_f64();
                if q > 0.0 {
                    vptrades.push_back((now, ev.price.to_f64(), q));
                }
            }
            _ => {}
        }
        // evict trades older than the lookback (no-lookahead rolling profile).
        let lo_ts = now.saturating_sub(l_ns);
        while let Some(&(ts, _, _)) = vptrades.front() {
            if ts < lo_ts { vptrades.pop_front(); } else { break; }
        }

        match state {
            St::Collecting => {
                if let Some(m) = micro_now {
                    let push = match cur_path.last() {
                        Some(&(t, _)) => now.saturating_sub(t) >= cfg.sample_ns,
                        None => true,
                    };
                    if push {
                        cur_path.push((now, m));
                    }
                }
                if now >= cur.end_ts {
                    let (class, rev_mag, cont_mag) =
                        classify_vp(&cur_path, cur.fire_ts, cur.dist_to_poc_bps, cur.long, cfg.class_thr_bps);
                    let revert_take = first_touch(&cur_path, cur.fire_ts, cur.long, cur.dist_to_poc_bps, cfg.stop_bps, cfg.hold_ns);
                    out.push(Det {
                        day: day.to_string(),
                        l_ns,
                        nbins,
                        lvn_frac,
                        fire_ts: cur.fire_ts,
                        long: cur.long,
                        entry_px: cur.entry_px,
                        poc_px: cur.poc_px,
                        dist_to_poc_bps: cur.dist_to_poc_bps,
                        lvn_vol_frac: cur.lvn_vol_frac,
                        class,
                        revert_mag: rev_mag,
                        cont_mag,
                        revert_take,
                    });
                    cur_path.clear();
                    state = St::Armed;
                }
            }
            St::Armed => {
                if profile.is_none() || now.saturating_sub(last_refresh) >= cfg.refresh_ns {
                    let snap: Vec<(u64, f64, f64)> = vptrades.iter().copied().collect();
                    profile = Profile::build(&snap, nbins, cfg.va_frac, cfg.min_trades);
                    last_refresh = now;
                }
                if now.saturating_sub(last_sample) >= cfg.sample_ns {
                    last_sample = now;
                    if now >= blocked_until {
                        if let (Some(m), Some(pf)) = (micro_now, profile.as_ref()) {
                            if m > 0.0 && pf.is_lvn(m, lvn_frac) && pf.outside_va(m) {
                                let dist = (pf.poc_px - m).abs() / m * 10_000.0;
                                if dist >= cfg.min_dist_bps {
                                    cur = PendingV {
                                        fire_ts: now,
                                        long: pf.poc_px > m,
                                        entry_px: m,
                                        poc_px: pf.poc_px,
                                        dist_to_poc_bps: dist,
                                        lvn_vol_frac: pf.vol_frac(m),
                                        end_ts: now.saturating_add(cfg.forward_ns),
                                    };
                                    cur_path.clear();
                                    cur_path.push((now, m));
                                    blocked_until = now.saturating_add(cfg.cooldown_ns);
                                    state = St::Collecting;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}
// ----------------------------------------------------------------------------
// reporting
// ----------------------------------------------------------------------------

/// One honest trade-stat line for realised signed-bps captures (net of `fee_bps`).
fn report_trades(label: &str, caps: &[f64], fee_bps: f64) {
    if caps.is_empty() {
        println!("  {label:<28}  (no entries)");
        return;
    }
    let n = caps.len();
    let wins = caps.iter().filter(|&&x| x > 0.0).count();
    let g = mean(caps);
    let worst = caps.iter().copied().fold(f64::INFINITY, f64::min);
    let win_mag = { let w: Vec<f64> = caps.iter().copied().filter(|&x| x > 0.0).collect(); mean(&w) };
    let loss_mag = { let l: Vec<f64> = caps.iter().copied().filter(|&x| x < 0.0).map(|x| -x).collect(); mean(&l) };
    let rr = if loss_mag > 0.0 { win_mag / loss_mag } else { 0.0 };
    println!(
        "  {label:<28}  n={n:<4} win {:>3.0}%  GROSS {:+.1}bps  t {:+.2}  worst {:+.0}  RR {:.2}  NET(-{:.1}) {:+.1}bps",
        wins as f64 / n as f64 * 100.0, g, tstat(caps), worst, rr, fee_bps, g - fee_bps
    );
}

fn caps_of<F: Fn(&Det) -> Option<f64>>(ds: &[&Det], f: F) -> Vec<f64> {
    ds.iter().filter_map(|s| f(s)).collect()
}

#[allow(clippy::too_many_lines)]
fn report_cell(label: &str, ds: &[&Det], cfg: &Cfg, num_days: usize) {
    println!("\n---------- {label} ----------");
    let n = ds.len();
    let per_day = if num_days > 0 { n as f64 / num_days as f64 } else { 0.0 };
    println!("LVN detections {n}   ({per_day:.1}/day over {num_days} days)");
    if n == 0 {
        println!("  (none - lvn-frac too tight, min-dist too wide, min-trades too high, or quiet)");
        return;
    }
    let longs = ds.iter().filter(|s| s.long).count();
    let dist: Vec<f64> = ds.iter().map(|s| s.dist_to_poc_bps).collect();
    let vf: Vec<f64> = ds.iter().map(|s| s.lvn_vol_frac).collect();
    println!(
        "direction(toward POC)         LONG(below value) {longs}  SHORT(above value) {}   dist-to-POC median {:.0}bps   node thinness median {:.2}xPOC",
        n - longs, median(&dist), median(&vf)
    );
    let rev = ds.iter().filter(|s| s.class == 1).count();
    let cont = ds.iter().filter(|s| s.class == -1).count();
    let neither = n - rev - cont;
    println!(
        "OUTCOME (reach POC vs run away)  REVERSION {}/{n} = {:.0}%   CONTINUATION {}/{n} = {:.0}%   neither {:.0}%",
        rev, rev as f64 / n as f64 * 100.0, cont, cont as f64 / n as f64 * 100.0, neither as f64 / n as f64 * 100.0
    );
    let rmag: Vec<f64> = ds.iter().filter(|s| s.class == 1).map(|s| s.revert_mag).collect();
    let cmag: Vec<f64> = ds.iter().filter(|s| s.class == -1).map(|s| s.cont_mag).collect();
    println!(
        "  reversion reach median {:.0}bps   continuation run median {:.0}bps   (target=POC dist, stop {:.0}bps)",
        median(&rmag), median(&cmag), cfg.stop_bps
    );
    println!("REVERSION trade (taker, enter at the LVN toward POC; run-overs IN, net 9bps):");
    report_trades("revert-TAKER (all)", &caps_of(ds, |s| s.revert_take), cfg.fee_bps);
    // split by node thinness (does a THINNER node revert better? = knob-bite probe).
    let med_vf = median(&vf);
    report_trades("revert-TAKER (thin<=med)", &caps_of(ds, |s| if s.lvn_vol_frac <= med_vf { s.revert_take } else { None }), cfg.fee_bps);
    // split by distance (is a FAR-from-value node a better fade?).
    let med_d = median(&dist);
    report_trades("revert-TAKER (far>=med dist)", &caps_of(ds, |s| if s.dist_to_poc_bps >= med_d { s.revert_take } else { None }), cfg.fee_bps);
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
        refresh_ns: 60_000_000_000,
        sample_ns: 5_000_000_000,
        forward_ns: 2 * 3_600_000_000_000,
        hold_ns: 0,
        cooldown_ns: 30 * 60_000_000_000,
        top_n: 5,
        va_frac: 0.70,
        class_thr_bps: 40.0,
        stop_bps: 40.0,
        min_dist_bps: 15.0,
        min_trades: 200,
        fee_bps: 6.0,
    };
    let mut dates: Vec<String> = Vec::new();
    let mut lookbacks: Vec<u64> = vec![2 * 3_600_000_000_000];
    let mut nbins_list: Vec<usize> = vec![50];
    let mut lvn_fracs: Vec<f64> = vec![0.20];
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
                cfg.hours = if h == "all" { (0..24).map(|x| format!("{x:02}")).collect() } else { h.split(',').map(|s| s.trim().to_string()).collect() };
            }
            "--lookbacks" => lookbacks = parse_dur_list(&val()?)?,
            "--nbins" => nbins_list = parse_usize_list(&val()?)?,
            "--lvn-frac" => lvn_fracs = parse_f64_list(&val()?)?,
            "--va-frac" => cfg.va_frac = val()?.parse().map_err(|e| format!("va-frac: {e}"))?,
            "--refresh" => cfg.refresh_ns = parse_dur(&val()?)?,
            "--sample" => cfg.sample_ns = parse_dur(&val()?)?,
            "--forward" => cfg.forward_ns = parse_dur(&val()?)?,
            "--cooldown" => cfg.cooldown_ns = parse_dur(&val()?)?,
            "--hold" => hold = Some(parse_dur(&val()?)?),
            "--top" => cfg.top_n = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--class-thr" => cfg.class_thr_bps = val()?.parse().map_err(|e| format!("class-thr: {e}"))?,
            "--stop" => cfg.stop_bps = val()?.parse().map_err(|e| format!("stop: {e}"))?,
            "--min-dist" => cfg.min_dist_bps = val()?.parse().map_err(|e| format!("min-dist: {e}"))?,
            "--min-trades" => cfg.min_trades = val()?.parse().map_err(|e| format!("min-trades: {e}"))?,
            "--fee" => cfg.fee_bps = val()?.parse().map_err(|e| format!("fee: {e}"))?,
            "--dump" => dump = Some(val()?),
            other => return Err(format!("unknown arg {other}")),
        }
    }
    if dates.is_empty() {
        return Err("missing --dates (comma list)".into());
    }
    cfg.hold_ns = hold.unwrap_or(cfg.forward_ns);

    println!("vpscope - volume-profile LVN mean-reversion study (analysis only, no trading)");
    println!("coin {}  dates {}", cfg.coin, dates.join(","));
    println!(
        "refresh {}  sample {}  forward {}  hold {}  cooldown {}  top {}  va-frac {:.2}",
        fmt_ns(cfg.refresh_ns), fmt_ns(cfg.sample_ns), fmt_ns(cfg.forward_ns), fmt_ns(cfg.hold_ns), fmt_ns(cfg.cooldown_ns), cfg.top_n, cfg.va_frac
    );
    println!(
        "trade: enter at LVN (outside value, dist-to-POC >= {:.0}bps) toward POC; target = POC dist; stop {:.0}bps; continuation if runs {:.0}bps away; min-trades/profile {}",
        cfg.min_dist_bps, cfg.stop_bps, cfg.class_thr_bps, cfg.min_trades
    );
    let lb_lbl: Vec<String> = lookbacks.iter().map(|&l| fmt_ns(l)).collect();
    println!("GRID: lookback {lb_lbl:?}  x  nbins {nbins_list:?}  x  lvn-frac {lvn_fracs:?}");

    let mut combos: Vec<(u64, usize, f64)> = Vec::new();
    for &l in &lookbacks {
        for &b in &nbins_list {
            for &f in &lvn_fracs {
                combos.push((l, b, f));
            }
        }
    }
    let mut pooled: Vec<Vec<Det>> = vec![Vec::new(); combos.len()];
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
            Err(e) => { eprintln!("  skip {day}: {e}"); continue; }
        };
        if evs.is_empty() { eprintln!("  skip {day}: empty"); continue; }
        eprintln!("  {day}: {} events", evs.len());
        for (ci, &(l, b, f)) in combos.iter().enumerate() {
            pooled[ci].extend(scan_day(&cfg, day, &evs, l, b, f));
        }
        loaded_days.push(day.clone());
    }
    let num_days = loaded_days.len();
    for (ci, &(l, b, f)) in combos.iter().enumerate() {
        println!("\n==================== L={}  nbins={b}  lvn-frac={f} ====================", fmt_ns(l));
        let all: Vec<&Det> = pooled[ci].iter().collect();
        report_cell("POOLED (all days)", &all, &cfg, num_days);
    }
    if let Some(path) = &dump {
        use std::io::Write;
        let mut fh = std::fs::File::create(path).map_err(|e| format!("dump: {e}"))?;
        writeln!(fh, "day,l_ns,nbins,lvn_frac,fire_ts,long,entry_px,poc_px,dist_to_poc_bps,lvn_vol_frac,class,revert_mag,cont_mag,revert_take").map_err(|e| format!("dump: {e}"))?;
        for col in &pooled {
            for c in col {
                writeln!(
                    fh,
                    "{},{},{},{},{},{},{:.4},{:.4},{:.2},{:.4},{},{:.2},{:.2},{}",
                    c.day, c.l_ns, c.nbins, c.lvn_frac, c.fire_ts, c.long, c.entry_px, c.poc_px,
                    c.dist_to_poc_bps, c.lvn_vol_frac, c.class, c.revert_mag, c.cont_mag,
                    c.revert_take.map_or("NA".to_string(), |x| format!("{x:.3}"))
                ).map_err(|e| format!("dump: {e}"))?;
            }
        }
        let total: usize = pooled.iter().map(Vec::len).sum();
        eprintln!("dumped {total} detections to {path}");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("{e}"); ExitCode::FAILURE }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn heavy_poc_thin_node() -> Vec<(u64, f64, f64)> {
        let mut t: Vec<(u64, f64, f64)> = Vec::new();
        for _ in 0..100 { t.push((0, 100.2, 1.0)); } // heavy POC bin
        for _ in 0..3 { t.push((0, 102.2, 1.0)); }    // thin node
        t.push((0, 99.0, 1.0)); // sets pmin
        t.push((0, 103.0, 1.0)); // sets pmax
        t
    }

    #[test]
    fn profile_poc_and_value_area() {
        let pf = Profile::build(&heavy_poc_thin_node(), 8, 0.70, 5).expect("builds");
        assert!((pf.poc_px - 100.25).abs() < 0.01, "POC at the heavy bin, got {}", pf.poc_px);
        // value area is tight around the dominant POC.
        assert!(pf.va_lo_px <= 100.25 && pf.va_hi_px >= 100.25, "VA brackets POC");
        assert!(pf.va_hi_px < 102.0, "thin node is OUTSIDE value, got va_hi {}", pf.va_hi_px);
    }

    #[test]
    fn lvn_and_outside_va_detection() {
        let pf = Profile::build(&heavy_poc_thin_node(), 8, 0.70, 5).expect("builds");
        assert!(pf.is_lvn(102.2, 0.20), "thin node is a low-volume node");
        assert!(!pf.is_lvn(100.2, 0.20), "the POC bin is NOT a low-volume node");
        assert!(pf.outside_va(102.2), "thin node sits outside the value area");
        assert!(!pf.outside_va(100.2), "the POC is inside the value area");
    }

    #[test]
    fn profile_none_when_too_few_trades() {
        let t = vec![(0u64, 100.0, 1.0), (0, 101.0, 1.0)];
        assert!(Profile::build(&t, 8, 0.70, 200).is_none(), "min-trades gate");
    }

    #[test]
    fn classify_reversion_reaches_poc() {
        // SHORT from 102 (poc below). Price falls to 100.9 -> favourable ~+108bps
        // >= dist_to_poc 100 -> REVERSION (+1).
        let path = vec![(0u64, 102.0), (1_000_000_000, 101.5), (2_000_000_000, 100.9)];
        let (class, fav, _adv) = classify_vp(&path, 0, 100.0, false, 40.0);
        assert_eq!(class, 1, "reached POC = reversion");
        assert!(fav > 100.0, "favourable excursion measured, got {fav}");
    }

    #[test]
    fn classify_continuation_runs_away() {
        // SHORT from 102 (poc below) but price RUNS UP to 103 -> adverse >= 40 ->
        // CONTINUATION (-1) (the thin node kept going = acceptance breakout).
        let path = vec![(0u64, 102.0), (1_000_000_000, 102.5), (2_000_000_000, 103.0)];
        let (class, _fav, adv) = classify_vp(&path, 0, 100.0, false, 40.0);
        assert_eq!(class, -1, "ran away = continuation");
        assert!(adv > 40.0, "adverse excursion measured, got {adv}");
    }

    #[test]
    fn first_touch_target_then_stop() {
        // LONG from 100, +50bps reached before any stop -> target 40 hit.
        let path = vec![(0u64, 100.0), (1_000_000_000, 100.3), (2_000_000_000, 100.5)];
        assert!(first_touch(&path, 0, true, 40.0, 30.0, 30_000_000_000).unwrap() >= 40.0);
        // SHORT from 100, price rises to 100.4 (+40bps against short) hits stop 30 first.
        let path2 = vec![(0u64, 100.0), (1_000_000_000, 100.4)];
        assert!(first_touch(&path2, 0, false, 100.0, 30.0, 30_000_000_000).unwrap() <= -30.0);
    }
}