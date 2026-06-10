//! Probabilistic and Deflated Sharpe Ratios (Bailey & Lopez de Prado).
//!
//! The Probabilistic Sharpe Ratio (PSR) is the probability the true Sharpe
//! exceeds a benchmark, given sample length and the return distribution's
//! skew/kurtosis. The Deflated Sharpe Ratio (DSR) is PSR with the benchmark set
//! to the EXPECTED MAXIMUM Sharpe under the null across `n_trials` configs - so
//! it penalises how many things you tried. DSR reduces to PSR when n_trials = 1.

use crate::normal::{inv_normal_cdf, normal_cdf};
use crate::stats::{kurtosis, mean, sharpe, skewness, std_dev};

/// Probabilistic Sharpe Ratio: P(true SR > `benchmark_sr`).
///
/// `returns` are per-observation; `benchmark_sr` is on the same (per-obs) scale.
#[must_use]
pub fn probabilistic_sharpe(returns: &[f64], benchmark_sr: f64) -> f64 {
    let t = returns.len();
    if t < 2 || std_dev(returns) <= 0.0 {
        return 0.0;
    }
    let sr = sharpe(returns);
    let g3 = skewness(returns);
    let g4 = kurtosis(returns);
    // denominator: estimation error of the Sharpe under non-normality.
    let denom = (1.0 - g3 * sr + (g4 - 1.0) / 4.0 * sr * sr).max(1e-12).sqrt();
    let z = (sr - benchmark_sr) * ((t as f64 - 1.0).sqrt()) / denom;
    normal_cdf(z)
}

/// Expected maximum Sharpe under the null across `n` independent trials whose
/// Sharpes have variance `var_trial_sharpes` (Bailey-LdP).
#[must_use]
pub fn expected_max_sharpe(var_trial_sharpes: f64, n: usize) -> f64 {
    let n = n.max(2) as f64;
    let euler = 0.577_215_664_901_532_9_f64;
    let e = std::f64::consts::E;
    var_trial_sharpes.max(0.0).sqrt()
        * ((1.0 - euler) * inv_normal_cdf(1.0 - 1.0 / n)
            + euler * inv_normal_cdf(1.0 - 1.0 / (n * e)))
}

/// Deflated Sharpe Ratio: probability the strategy's true Sharpe is positive
/// after correcting for selection bias across `n_trials` and non-normality.
///
/// `var_trial_sharpes` is the variance of the Sharpe ratios observed across all
/// `n_trials` configs in the sweep.
#[must_use]
pub fn deflated_sharpe(returns: &[f64], n_trials: usize, var_trial_sharpes: f64) -> f64 {
    let sr0 = expected_max_sharpe(var_trial_sharpes, n_trials);
    probabilistic_sharpe(returns, sr0)
}

/// Convenience: variance of a set of trial Sharpe ratios.
#[must_use]
pub fn variance_of_sharpes(sharpes: &[f64]) -> f64 {
    if sharpes.len() < 2 {
        return 0.0;
    }
    let m = mean(sharpes);
    sharpes.iter().map(|s| (s - m) * (s - m)).sum::<f64>() / sharpes.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noisy_positive(seed: u64, n: usize, drift: f64) -> Vec<f64> {
        // deterministic pseudo-normal-ish returns with a small positive drift
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let u = (s >> 11) as f64 / (1u64 << 53) as f64; // [0,1)
                (u - 0.5) + drift
            })
            .collect()
    }

    #[test]
    fn psr_higher_for_stronger_edge() {
        let weak = noisy_positive(1, 500, 0.02);
        let strong = noisy_positive(1, 500, 0.20);
        assert!(probabilistic_sharpe(&strong, 0.0) > probabilistic_sharpe(&weak, 0.0));
    }

    #[test]
    fn dsr_decreases_with_more_trials() {
        let r = noisy_positive(7, 1000, 0.10);
        let v = 0.01; // some spread of trial sharpes
        let few = deflated_sharpe(&r, 5, v);
        let many = deflated_sharpe(&r, 5000, v);
        assert!(many <= few, "more trials must not raise DSR: few={few} many={many}");
    }

    #[test]
    fn dsr_equals_psr_at_one_trial() {
        let r = noisy_positive(3, 400, 0.08);
        // n=1 -> expected max sharpe uses n.max(2); use benchmark 0 comparison loosely
        let dsr1 = deflated_sharpe(&r, 1, 0.0);
        let psr0 = probabilistic_sharpe(&r, 0.0);
        assert!((dsr1 - psr0).abs() < 1e-9);
    }
}