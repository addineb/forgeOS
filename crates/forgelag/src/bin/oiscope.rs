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
    let mut cur = Pending { fire_ts: 0, dir_down: false, baseline: 0.0, oi_drop_pct: 0.0, price_move_bps: 0.0, net_flow: 0.0, end_ts: 0 };
    let mut cur_path: Vec<(u64, f64)> = Vec::new();
    let mut cur_opath: Vec<(u64, f64)> = Vec::new();
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
                if now >= cur.end_ts {
                    out.push(finalize(&cur, &cur_path, &cur_opath, cfg, day, d_oi, p_move));
                    cur_path.clear();
                    cur_opath.clear();
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
                                };
                                cur_path.clear();
                                cur_opath.clear();
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
fn finalize(cur: &Pending, path: &[(u64, f64)], oi_path: &[(u64, f64)], cfg: &Cfg, day: &str, d_oi: f64, p_move: f64) -> CascadeStat {
    let rev = measure_reversion(path, cur.fire_ts, cur.baseline, cur.dir_down);
    let reverted = rev.revert_frac >= cfg.revert_frac;
    // Choose the fade ENTRY ANCHOR: baseline fades at the fire; --exhaust waits for
    // the exhaustion point (forced flow runs out) and only then fades. If exhaustion
    // never fires within the horizon, the cascade is SKIPPED (no entries).
    let exhaust_ts = if cfg.exhaust {
        find_exhaustion(path, oi_path, cur.fire_ts, cur.dir_down, cfg.exhaust_stall_ns, cfg.exhaust_decel)
    } else {
        Some(cur.fire_ts)
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
        let mut hdr = String::from("day,oi_drop_thr,price_move_thr,fire_ts,dir_down,oi_drop_pct,price_move_bps,net_flow,spike_bps,revert_bps,revert_frac,reverted,full_revert,exhausted,exhaust_ms,tth_ms,ttf_ms,peak_ms");
        for &dl in &cfg.delays_ns {
            hdr.push_str(&format!(",fade_{}", fmt_ns(dl)));
        }
        writeln!(f, "{hdr}").map_err(|e| format!("dump: {e}"))?;
        for col in &pooled {
            for c in col {
                let mut line = format!(
                    "{},{},{},{},{},{:.4},{:.3},{:.4},{:.3},{:.3},{:.4},{},{},{},{},{},{},{:.0}",
                    c.day, c.d_oi, c.p_move, c.fire_ts, c.dir_down, c.oi_drop_pct, c.price_move_bps, c.net_flow,
                    c.spike_bps, c.revert_bps, c.revert_frac, c.reverted, c.full_revert,
                    c.exhausted,
                    c.exhaust_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.tth_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.ttf_ns.map_or(-1.0, |x| x as f64 / 1e6),
                    c.peak_ns as f64 / 1e6
                );
                for fv in &c.fades {
                    match fv {
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
}