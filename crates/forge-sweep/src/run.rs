//! The parallel sweep runner (generic over strategy), scoring, and verdict.

use rayon::prelude::*;

use forge_core::Event;
use forge_metrics::{deflated_sharpe, pbo_cscv, sharpe, variance_of_sharpes};
use forge_sim::{money_to_f64, FeeSchedule, SimConfig, SimEngine, Strategy};

/// Tunable two-bar thresholds. Lenient to KEEP (park), strict to PROMOTE.
#[derive(Clone, Copy, Debug)]
pub struct Thresholds {
    /// Minimum completed round trips to be taken seriously.
    pub min_round_trips: u64,
    /// DSR needed to keep a config as a candidate (park).
    pub dsr_candidate: f64,
    /// DSR needed to send a config live (promote).
    pub dsr_live: f64,
    /// Sweep PBO at/above which nothing is promoted (overfit territory).
    pub pbo_max: f64,
    /// Sweep PBO required to promote anything.
    pub pbo_live: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { min_round_trips: 30, dsr_candidate: 0.60, dsr_live: 0.90, pbo_max: 0.50, pbo_live: 0.30 }
    }
}

/// The fate of a config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Clears the strict bar; eligible to go live.
    Promote,
    /// Shows a pulse; keep refining / gather more data.
    Park,
    /// Net-negative, untraded, or indistinguishable from noise.
    Retire,
}

/// Per-config result + score. Generic over the config type `C`.
#[derive(Clone, Debug)]
pub struct CellResult<C> {
    /// Config id (stable across runs and thread counts).
    pub id: usize,
    /// The config.
    pub config: C,
    /// Net P&L summed across windows (quote units).
    pub net: f64,
    /// Completed round trips across windows.
    pub round_trips: u64,
    /// Fraction of round trips that were profitable.
    pub win_rate: f64,
    /// Mean per-trade return in percent of notional (the leverage-free edge).
    pub avg_pct: f64,
    /// Worst peak-to-trough equity drawdown across windows (quote units).
    pub max_dd: f64,
    /// Sharpe of the per-bucket equity returns.
    pub sharpe: f64,
    /// Deflated Sharpe (probability of real edge given the trial count).
    pub dsr: f64,
    /// Verdict.
    pub verdict: Verdict,
    /// Aligned per-bucket equity returns (used for PBO / DSR).
    returns: Vec<f64>,
}

/// The whole sweep.
#[derive(Clone, Debug)]
pub struct SweepReport<C> {
    /// Cells in config-id order.
    pub cells: Vec<CellResult<C>>,
    /// Sweep-level Probability of Backtest Overfitting (None if not computable).
    pub pbo: Option<f64>,
    /// Number of configs tried (the multiple-testing count).
    pub n_trials: usize,
    /// Config id with the highest net.
    pub best_net_id: Option<usize>,
}

fn choose_blocks(t: usize) -> Option<usize> {
    [16usize, 12, 10, 8, 6, 4].into_iter().find(|&s| t >= s)
}

fn decide<C>(c: &CellResult<C>, sweep_pbo: Option<f64>, th: &Thresholds) -> Verdict {
    if c.net <= 0.0 || c.round_trips < th.min_round_trips || c.dsr < 0.5 {
        return Verdict::Retire;
    }
    let pbo = sweep_pbo.unwrap_or(1.0);
    if c.dsr >= th.dsr_live && pbo <= th.pbo_live {
        return Verdict::Promote;
    }
    if c.dsr >= th.dsr_candidate && pbo <= th.pbo_max {
        return Verdict::Park;
    }
    Verdict::Retire
}

/// Run the full sweep. `configs` are the grid cells; `make` turns a config into
/// a strategy. Generic over the config type and strategy, so any bot can be
/// swept. Deterministic and identical at any thread count.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn run_sweep<C, S, F>(
    windows: &[Vec<Event>],
    configs: &[C],
    make: F,
    sample_ns: u64,
    fees: FeeSchedule,
    latency_ns: u64,
    book_levels: usize,
    th: Thresholds,
) -> SweepReport<C>
where
    C: Copy + Send + Sync,
    S: Strategy,
    F: Fn(C) -> S + Send + Sync,
{
    let n = configs.len();

    let mut cells: Vec<CellResult<C>> = (0..n)
        .into_par_iter()
        .map(|id| {
            let cfg = configs[id];
            let mut returns = Vec::new();
            let mut net = 0.0f64;
            let mut trips = 0u64;
            let mut wins = 0u64;
            let mut trip_total = 0u64;
            let mut pct_sum = 0.0f64;
            let mut pct_count = 0u64;
            let mut max_dd = 0.0f64;

            for w in windows {
                let sim_cfg = SimConfig { order_latency_ns: latency_ns, book_max_levels: book_levels, fees };
                let mut eng = SimEngine::new(make(cfg), sim_cfg);
                eng.enable_equity_sampling(sample_ns);
                eng.run(w.iter()).expect("monotonic event stream");

                let curve = eng.equity_curve();
                for pair in curve.windows(2) {
                    returns.push(money_to_f64(pair[1].1 - pair[0].1));
                }
                let mut peak = i128::MIN;
                for &(_, eq) in curve {
                    if eq > peak {
                        peak = eq;
                    }
                    let dd = money_to_f64(peak - eq);
                    if dd > max_dd {
                        max_dd = dd;
                    }
                }

                let tp = eng.account().trip_pnls();
                let tn = eng.account().trip_notionals();
                for (pnl, notl) in tp.iter().zip(tn.iter()) {
                    if *pnl > 0 {
                        wins += 1;
                    }
                    if *notl > 0 {
                        pct_sum += money_to_f64(*pnl) / money_to_f64(*notl) * 100.0;
                        pct_count += 1;
                    }
                }
                trip_total += tp.len() as u64;

                let r = eng.finish();
                net += money_to_f64(r.net_pnl);
                trips += r.round_trips;
            }

            let shp = sharpe(&returns);
            let win_rate = if trip_total == 0 { 0.0 } else { wins as f64 / trip_total as f64 };
            let avg_pct = if pct_count == 0 { 0.0 } else { pct_sum / pct_count as f64 };
            CellResult {
                id,
                config: cfg,
                net,
                round_trips: trips,
                win_rate,
                avg_pct,
                max_dd,
                sharpe: shp,
                dsr: 0.0,
                verdict: Verdict::Retire,
                returns,
            }
        })
        .collect();

    let pbo = if n >= 2 {
        let t = cells.iter().map(|c| c.returns.len()).min().unwrap_or(0);
        match choose_blocks(t) {
            Some(s) => {
                let matrix: Vec<Vec<f64>> = cells.iter().map(|c| c.returns[..t].to_vec()).collect();
                pbo_cscv(&matrix, s).map(|r| r.pbo)
            }
            None => None,
        }
    } else {
        None
    };

    let var = variance_of_sharpes(&cells.iter().map(|c| c.sharpe).collect::<Vec<_>>());
    for c in cells.iter_mut() {
        c.dsr = deflated_sharpe(&c.returns, n, var);
        c.verdict = decide(c, pbo, &th);
    }

    let best_net_id = cells
        .iter()
        .max_by(|a, b| a.net.partial_cmp(&b.net).unwrap_or(std::cmp::Ordering::Equal))
        .map(|c| c.id);

    SweepReport { cells, pbo, n_trials: n, best_net_id }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{EventKind, Price, Qty, Side, UnixNanos};
    use forge_strategy::{MomentumConfig, OfiMomentum, Signal};

    struct Rng(u64);
    impl Rng {
        fn bit(&mut self) -> bool {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 33) & 1 == 1
        }
    }

    fn synth(seed: u64, ticks: usize) -> Vec<Event> {
        let mut rng = Rng(seed | 1);
        let mut mid: i64 = 100 * 100_000_000;
        let half = 1_000_000;
        let tick = 1_000_000;
        let size = Qty::from_f64(100.0).unwrap().raw();
        let mut prev: Option<(i64, i64)> = None;
        let mut ts = 1_000_000u64;
        let push = |out: &mut Vec<Event>, s: Side, px: i64, q: i64, ts: u64| {
            out.push(Event::new(EventKind::BookDelta, UnixNanos::new(ts), UnixNanos::new(ts), Some(s), Price::from_raw(px), Qty::from_raw(q), 0).unwrap());
        };
        let mut out = Vec::new();
        for i in 0..ticks {
            ts += 1_000_000;
            if i > 0 {
                mid += if rng.bit() { tick } else { -tick };
            }
            let bid = mid - half;
            let ask = mid + half;
            push(&mut out, Side::Bid, bid, size, ts);
            push(&mut out, Side::Ask, ask, size, ts);
            if let Some((pb, pa)) = prev {
                if pb != bid { push(&mut out, Side::Bid, pb, 0, ts); }
                if pa != ask { push(&mut out, Side::Ask, pa, 0, ts); }
            }
            prev = Some((bid, ask));
        }
        out
    }

    fn small_grid() -> Vec<MomentumConfig> {
        crate::expand(&crate::GridSpec {
            ofi_window: vec![5, 10],
            threshold: vec![0.3, 0.6],
            qty: Qty::from_f64(0.1).unwrap(),
            hold_ns: vec![3_000_000],
            cooldown_ns: vec![1_000_000],
            tp_bps: vec![0.0],
            sl_bps: vec![0.0],
            use_limit: vec![false],
            signal: Signal::Real,
            seed: 1,
            fill_timeout_ns: 200_000_000,
        })
    }

    #[test]
    fn sweep_is_deterministic_across_thread_counts() {
        let windows = vec![synth(0xABCD, 4_000)];
        let grid = small_grid();
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        let a = one.install(|| run_sweep(&windows, &grid, OfiMomentum::new, 1_000_000, FeeSchedule::legacy(), 0, 20, Thresholds::default()));
        let b = many.install(|| run_sweep(&windows, &grid, OfiMomentum::new, 1_000_000, FeeSchedule::legacy(), 0, 20, Thresholds::default()));
        assert_eq!(a.cells.len(), b.cells.len());
        for (x, y) in a.cells.iter().zip(b.cells.iter()) {
            assert_eq!(x.id, y.id);
            assert!((x.net - y.net).abs() < 1e-9);
            assert!((x.dsr - y.dsr).abs() < 1e-12);
        }
        assert_eq!(a.pbo, b.pbo);
    }

    #[test]
    fn random_data_yields_no_promotions() {
        let windows = vec![synth(0x1234, 6_000)];
        let grid = small_grid();
        let rep = run_sweep(&windows, &grid, OfiMomentum::new, 1_000_000, FeeSchedule::legacy(), 0, 20, Thresholds::default());
        assert!(rep.cells.iter().all(|c| c.verdict != Verdict::Promote));
    }
}