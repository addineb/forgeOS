//! Cartesian-product expansion of a parameter grid into concrete configs.

use forge_core::Qty;
use forge_strategy::{MomentumConfig, Signal};

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
    /// Hold lengths (events) to try.
    pub hold: Vec<u32>,
    /// Cooldowns (events) to try.
    pub cooldown: Vec<u32>,
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
            for &h in &spec.hold {
                for &cd in &spec.cooldown {
                    for &tp in &spec.tp_bps {
                        for &sl in &spec.sl_bps {
                            for &lim in &spec.use_limit {
                                out.push(MomentumConfig {
                                    ofi_window: w,
                                    threshold: th,
                                    qty: spec.qty,
                                    hold: h,
                                    cooldown: cd,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartesian_size_and_order() {
        let spec = GridSpec {
            ofi_window: vec![10, 20],
            threshold: vec![1.0, 2.0, 3.0],
            qty: Qty::from_raw(1_000_000),
            hold: vec![50],
            cooldown: vec![10],
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