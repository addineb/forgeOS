//! Cartesian-product expansion of a parameter grid into concrete configs.

use forge_core::Qty;
use forge_strategy::{CvdConfig, ImbalanceConfig, MomentumConfig, Signal};

/// The swept knobs. Each vector is a set of values to try; the grid is their
/// Cartesian product (deterministic order).
#[derive(Clone, Debug)]
pub struct GridSpec {
    /// OFI windows to try.
    pub ofi_window: Vec<usize>,
    /// Entry thresholds to try.
    pub threshold: Vec<f64>,
    /// Trade size (fixed).
    pub qty: Qty,
    /// Hold durations (nanoseconds) to try.
    pub hold_ns: Vec<u64>,
    /// Cooldowns (nanoseconds) to try.
    pub cooldown_ns: Vec<u64>,
    /// Take-profit bps to try (0 = disabled).
    pub tp_bps: Vec<f64>,
    /// Stop-loss bps to try (0 = disabled).
    pub sl_bps: Vec<f64>,
    /// Market (false) vs limit (true) entry.
    pub use_limit: Vec<bool>,
    /// Direction source (Real for the thesis; Shuffled for the control).
    pub signal: Signal,
    /// Seed (for the shuffled control + tie-breaking).
    pub seed: u64,
    /// Fill timeout.
    pub fill_timeout_ns: u64,
}

/// Expand the grid into concrete `MomentumConfig`s in a fixed, deterministic
/// order (so config ids are stable run to run).
#[must_use]
pub fn expand(spec: &GridSpec) -> Vec<MomentumConfig> {
    let mut out = Vec::new();
    for &w in &spec.ofi_window {
        for &th in &spec.threshold {
            for &h in &spec.hold_ns {
                for &cd in &spec.cooldown_ns {
                    for &tp in &spec.tp_bps {
                        for &sl in &spec.sl_bps {
                            for &lim in &spec.use_limit {
                                out.push(MomentumConfig {
                                    ofi_window: w,
                                    threshold: th,
                                    qty: spec.qty,
                                    hold_ns: h,
                                    cooldown_ns: cd,
                                    tp_bps: tp,
                                    sl_bps: sl,
                                    use_limit: lim,
                                    signal: spec.signal,
                                    seed: spec.seed,
                                    fill_timeout_ns: spec.fill_timeout_ns,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Grid for the order-book-imbalance / wall bot.
#[derive(Clone, Debug)]
pub struct ImbalanceGridSpec {
    /// Top-N levels to sum.
    pub top_n: Vec<usize>,
    /// Imbalance thresholds.
    pub threshold: Vec<f64>,
    /// Follow (false) vs fade (true) the wall.
    pub reversion: Vec<bool>,
    /// Trade size.
    pub qty: Qty,
    /// Hold durations (nanoseconds).
    pub hold_ns: Vec<u64>,
    /// Cooldowns (nanoseconds).
    pub cooldown_ns: Vec<u64>,
    /// Take-profit bps.
    pub tp_bps: Vec<f64>,
    /// Stop-loss bps.
    pub sl_bps: Vec<f64>,
    /// Market vs limit entry.
    pub use_limit: Vec<bool>,
    /// Direction source.
    pub signal: Signal,
    /// Seed.
    pub seed: u64,
    /// Fill timeout.
    pub fill_timeout_ns: u64,
}

/// Expand the imbalance grid into concrete configs (deterministic order).
#[must_use]
pub fn expand_imbalance(spec: &ImbalanceGridSpec) -> Vec<ImbalanceConfig> {
    let mut out = Vec::new();
    for &tn in &spec.top_n {
        for &th in &spec.threshold {
            for &rev in &spec.reversion {
                for &h in &spec.hold_ns {
                    for &cd in &spec.cooldown_ns {
                        for &tp in &spec.tp_bps {
                            for &sl in &spec.sl_bps {
                                for &lim in &spec.use_limit {
                                    out.push(ImbalanceConfig {
                                        top_n: tn,
                                        threshold: th,
                                        reversion: rev,
                                        qty: spec.qty,
                                        hold_ns: h,
                                        cooldown_ns: cd,
                                        tp_bps: tp,
                                        sl_bps: sl,
                                        use_limit: lim,
                                        signal: spec.signal,
                                        seed: spec.seed,
                                        fill_timeout_ns: spec.fill_timeout_ns,
                                    });
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

/// Grid for the CVD bot.
#[derive(Clone, Debug)]
pub struct CvdGridSpec {
    /// Rolling windows (trades).
    pub window: Vec<usize>,
    /// Thresholds.
    pub threshold: Vec<f64>,
    /// Follow (false) vs fade (true).
    pub reversion: Vec<bool>,
    /// Trade size.
    pub qty: Qty,
    /// Hold durations (nanoseconds).
    pub hold_ns: Vec<u64>,
    /// Cooldowns (nanoseconds).
    pub cooldown_ns: Vec<u64>,
    /// Take-profit bps.
    pub tp_bps: Vec<f64>,
    /// Stop-loss bps.
    pub sl_bps: Vec<f64>,
    /// Market vs limit entry.
    pub use_limit: Vec<bool>,
    /// Direction source.
    pub signal: Signal,
    /// Seed.
    pub seed: u64,
    /// Fill timeout.
    pub fill_timeout_ns: u64,
}

/// Expand the CVD grid into concrete configs (deterministic order).
#[must_use]
pub fn expand_cvd(spec: &CvdGridSpec) -> Vec<CvdConfig> {
    let mut out = Vec::new();
    for &w in &spec.window {
        for &th in &spec.threshold {
            for &rev in &spec.reversion {
                for &h in &spec.hold_ns {
                    for &cd in &spec.cooldown_ns {
                        for &tp in &spec.tp_bps {
                            for &sl in &spec.sl_bps {
                                for &lim in &spec.use_limit {
                                    out.push(CvdConfig {
                                        window: w,
                                        threshold: th,
                                        reversion: rev,
                                        qty: spec.qty,
                                        hold_ns: h,
                                        cooldown_ns: cd,
                                        tp_bps: tp,
                                        sl_bps: sl,
                                        use_limit: lim,
                                        signal: spec.signal,
                                        seed: spec.seed,
                                        fill_timeout_ns: spec.fill_timeout_ns,
                                    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartesian_size_and_order() {
        let spec = GridSpec {
            ofi_window: vec![10, 20],
            threshold: vec![1.0, 2.0, 3.0],
            qty: Qty::from_raw(1_000_000),
            hold_ns: vec![5_000_000_000],
            cooldown_ns: vec![1_000_000_000],
            tp_bps: vec![0.0],
            sl_bps: vec![0.0],
            use_limit: vec![false, true],
            signal: Signal::Real,
            seed: 1,
            fill_timeout_ns: 200_000_000,
        };
        let g = expand(&spec);
        assert_eq!(g.len(), 12); // 2 windows x 3 thresholds x 2 use_limit
        // first cell uses the first value of each axis
        assert_eq!(g[0].ofi_window, 10);
        assert_eq!(g[0].threshold, 1.0);
        assert!(!g[0].use_limit);
    }
}