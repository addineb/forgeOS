//! The parallel sweep runner, scoring, and verdict.

use rayon::prelude::*;

use forge_core::Event;
use forge_metrics::{deflated_sharpe, pbo_cscv, sharpe, variance_of_sharpes};
use forge_sim::{money_to_f64, FeeSchedule, SimConfig, SimEngine};
use forge_strategy::{MomentumConfig, OfiMomentum};

/// Tunable two-bar thresholds. Lenient to KEEP (park), strict to PROMOTE.
#[derive(Clone, Copy, Debug)]
pub struct Thresholds {
    /// Minimum completed round trips to be taken seriously (knob must trade).
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

/// Per-config result + score.
#[derive(Clone, Debug)]
pub struct CellResult {
    /// Config id (stable across runs and thread counts).
    pub id: usize,
    /// The config.
    pub config: MomentumConfig,
    /// Net P&L summed across windows (quote units).
    pub net: f64,
    /// Completed round trips across windows.
    pub round_trips: u64,
    /// Fraction of round trips that were profitable.
    pub win_rate: f64,
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
pub struct SweepReport {
    /// Cells in config-id order.
    pub cells: Vec<CellResult>,
    /// Sweep-level Probability of Backtest Overfitting (None if not computable).
    pub pbo: Option<f64>,
    /// Number of configs tried (the multiple-testing count).
    pub n_trials: usize,
    /// Config id with the highest net (the in-sample "winner").
    pub best_net_id: Option<usize>,
}

/// Pick an even CSCV block count that fits the observation length.
fn choose_blocks(t: usize) -> Option<usize> {
    [16usize, 12, 10, 8, 6, 4].into_iter().find(|&s| t >= s)
}

fn decide(c: &CellResult, sweep_pbo: Option<f64>, th: &Thresholds) -> Verdict {
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

/// Run the full sweep. `windows` are pre-decoded event streams (one per window);
/// `grid` is the config list; `sample_ns` is the equity-curve bucket size.
#[must_use]
pub fn run_sweep(
    windows: &[Vec<Event>],
    grid: &[MomentumConfig],
    sample_ns: u64,
    fees: FeeSchedule,
    latency_ns: u64,
    book_levels: usize,
    th: Thresholds,
) -> SweepReport {
    let n = grid.len();

    // Parallel over configs; rayon preserves order in collect, and each run is
    // an independent deterministic sim -> identical results at any thread count.
    let mut cells: Vec<CellResult> = (0..n)
        .into_par_iter()
        .map(|id| {
            let cfg = grid[id];
            let mut returns = Vec::new();
            let mut net = 0.0f64;
            let mut trips = 0u64;
            let mut wins = 0u64;
            let mut trip_total = 0u64;

            for w in windows {
                let sim_cfg = SimConfig {
                    order_latency_ns: latency_ns,
                    book_max_levels: book_levels,
                    fees,
                };
                let mut eng = SimEngine::new(OfiMomentum::new(cfg), sim_cfg);
                eng.enable_equity_sampling(sample_ns);
                eng.run(w.iter()).expect("monotonic event stream");

                for pair in eng.equity_curve().windows(2) {
                    returns.push(money_to_f64(pair[1].1 - pair[0].1));
                }
                let tp = eng.account().trip_pnls();
                wins += tp.iter().filter(|&&p| p > 0).count() as u64;
                trip_total += tp.len() as u64;

                let r = eng.finish();
                net += money_to_f64(r.net_pnl);
                trips += r.round_trips;
            }

            let shp = sharpe(&returns);
            let win_rate = if trip_total == 0 { 0.0 } else { wins as f64 / trip_total as f64 };
            CellResult {
                id,
                config: cfg,
                net,
                round_trips: trips,
                win_rate,
                sharpe: shp,
                dsr: 0.0,
                verdict: Verdict::Retire,
                returns,
            }
        })
        .collect();

    // Sweep-level PBO over the aligned per-bucket return matrix.
    let pbo = if n >= 2 {
        let t = cells.iter().map(|c| c.returns.len()).min().unwrap_or(0);
        match choose_blocks(t) {
            Some(s) => {
                // truncate all rows to the common length t so the matrix aligns
                let matrix: Vec<Vec<f64>> =
                    cells.iter().map(|c| c.returns[..t].to_vec()).collect();
                pbo_cscv(&matrix, s).map(|r| r.pbo)
            }
            None => None,
        }
    } else {
        None
    };

    // DSR per cell against the trial distribution, then verdict.
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
    use forge_strategy::Signal;

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
        let mut out = Vec::new();
        let push = |out: &mut Vec<Event>, s: Side, px: i64, q: i64, ts: u64| {
            out.push(
                Event::new(EventKind::BookDelta, UnixNanos::new(ts), UnixNanos::new(ts), Some(s), Price::from_raw(px), Qty::from_raw(q), 0).unwrap(),
            );
        };
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
            hold: vec![3],
            cooldown: vec![1],
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
        let a = one.install(|| run_sweep(&windows, &grid, 1_000_000, FeeSchedule::legacy(), 0, 20, Thresholds::default()));
        let b = many.install(|| run_sweep(&windows, &grid, 1_000_000, FeeSchedule::legacy(), 0, 20, Thresholds::default()));
        assert_eq!(a.cells.len(), b.cells.len());
        for (x, y) in a.cells.iter().zip(b.cells.iter()) {
            assert_eq!(x.id, y.id);
            assert!((x.net - y.net).abs() < 1e-9, "net differs across thread counts");
            assert!((x.dsr - y.dsr).abs() < 1e-12);
        }
        assert_eq!(a.pbo, b.pbo);
    }

    #[test]
    fn random_data_yields_no_promotions() {
        // OFI momentum on a random walk: nothing should clear the live bar.
        let windows = vec![synth(0x1234, 6_000)];
        let grid = small_grid();
        let rep = run_sweep(&windows, &grid, 1_000_000, FeeSchedule::legacy(), 0, 20, Thresholds::default());
        assert!(rep.cells.iter().all(|c| c.verdict != Verdict::Promote), "random data must not promote");
    }
}