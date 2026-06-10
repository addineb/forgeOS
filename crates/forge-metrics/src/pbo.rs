//! Probability of Backtest Overfitting via Combinatorially-Symmetric
//! Cross-Validation (Bailey, Borwein, Lopez de Prado & Zhu).
//!
//! Split the observation axis into `s` equal blocks. For every way to choose
//! `s/2` blocks as in-sample (the rest out-of-sample): find the config that
//! looked best in-sample (max Sharpe), then measure its out-of-sample rank
//! among all configs. If the in-sample winner is below the OOS median, that
//! combination shows overfitting. PBO = the fraction of combinations where the
//! IS winner lands below the OOS median. PBO < 0.5 is the bar.

use crate::stats::sharpe;

/// Result of a CSCV run.
#[derive(Clone, Debug, PartialEq)]
pub struct PboResult {
    /// Probability of backtest overfitting (fraction of combos where the IS
    /// winner is below the OOS median).
    pub pbo: f64,
    /// Number of CSCV combinations evaluated.
    pub combinations: usize,
    /// Median out-of-sample logit (>0 = IS winner tends to stay above median).
    pub median_logit: f64,
}

/// All `k`-subsets of `0..s` as bitmasks.
fn combinations(s: usize, k: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..k).collect();
    if k == 0 || k > s {
        return out;
    }
    loop {
        let mut mask = 0u32;
        for &i in &idx {
            mask |= 1 << i;
        }
        out.push(mask);
        // advance the combination
        let mut i = k;
        while i > 0 {
            i -= 1;
            if idx[i] != i + s - k {
                idx[i] += 1;
                for j in i + 1..k {
                    idx[j] = idx[j - 1] + 1;
                }
                break;
            }
            if i == 0 {
                return out;
            }
        }
    }
}

fn sharpe_over_blocks(series: &[f64], blocks: &[(usize, usize)], mask: u32) -> f64 {
    let mut vals = Vec::new();
    for (b, &(lo, hi)) in blocks.iter().enumerate() {
        if mask & (1 << b) != 0 {
            vals.extend_from_slice(&series[lo..hi]);
        }
    }
    sharpe(&vals)
}

/// Run CSCV. `returns[i]` is config `i`'s per-observation return series; all
/// series must share the same length `t`. `s` is the (even) number of blocks.
///
/// Returns `None` if inputs are malformed (`< 2` configs, odd or `< 2` `s`,
/// `t < s`, or ragged series).
#[must_use]
pub fn pbo_cscv(returns: &[Vec<f64>], s: usize) -> Option<PboResult> {
    let n = returns.len();
    if n < 2 || s < 2 || !s.is_multiple_of(2) {
        return None;
    }
    let t = returns[0].len();
    if t < s || returns.iter().any(|r| r.len() != t) {
        return None;
    }

    // contiguous block boundaries
    let blocks: Vec<(usize, usize)> = (0..s).map(|b| (b * t / s, (b + 1) * t / s)).collect();
    let half = s / 2;
    let full: u32 = if s == 32 { u32::MAX } else { (1u32 << s) - 1 };

    let mut logits = Vec::new();
    for is_mask in combinations(s, half) {
        let oos_mask = full & !is_mask;

        // in-sample winner = argmax IS Sharpe
        let mut best = 0usize;
        let mut best_sr = f64::NEG_INFINITY;
        for (i, series) in returns.iter().enumerate() {
            let sr = sharpe_over_blocks(series, &blocks, is_mask);
            if sr > best_sr {
                best_sr = sr;
                best = i;
            }
        }

        // out-of-sample Sharpes; relative rank of the winner
        let win_oos = sharpe_over_blocks(&returns[best], &blocks, oos_mask);
        let worse = returns
            .iter()
            .filter(|series| sharpe_over_blocks(series, &blocks, oos_mask) <= win_oos)
            .count();
        let omega = (worse as f64 / (n as f64 + 1.0)).clamp(1e-6, 1.0 - 1e-6);
        logits.push((omega / (1.0 - omega)).ln());
    }

    if logits.is_empty() {
        return None;
    }
    let pbo = logits.iter().filter(|&&l| l <= 0.0).count() as f64 / logits.len() as f64;
    let mut sorted = logits.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_logit = sorted[sorted.len() / 2];

    Some(PboResult { pbo, combinations: logits.len(), median_logit })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xorshift(s: &mut u64) -> f64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        (*s >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    }

    #[test]
    fn combinations_count() {
        // C(4,2) = 6
        assert_eq!(combinations(4, 2).len(), 6);
        // C(6,3) = 20
        assert_eq!(combinations(6, 3).len(), 20);
    }

    #[test]
    fn pure_noise_is_overfit_prone() {
        // many noise configs -> the IS winner is luck -> PBO near 0.5
        let mut s = 12345u64;
        let returns: Vec<Vec<f64>> =
            (0..40).map(|_| (0..400).map(|_| xorshift(&mut s)).collect()).collect();
        let r = pbo_cscv(&returns, 8).unwrap();
        assert!(r.pbo > 0.3, "noise PBO should be high, got {}", r.pbo);
    }

    #[test]
    fn one_genuinely_better_config_lowers_pbo() {
        // config 0 has a consistent positive drift; others are noise.
        let mut s = 999u64;
        let mut returns: Vec<Vec<f64>> =
            (0..20).map(|_| (0..400).map(|_| xorshift(&mut s)).collect()).collect();
        for v in returns[0].iter_mut() {
            *v += 0.5; // strong, persistent edge in every block
        }
        let r = pbo_cscv(&returns, 8).unwrap();
        assert!(r.pbo < 0.1, "a real persistent edge should give low PBO, got {}", r.pbo);
    }

    #[test]
    fn rejects_malformed() {
        assert!(pbo_cscv(&[vec![1.0; 10]], 4).is_none()); // <2 configs
        assert!(pbo_cscv(&[vec![1.0; 10], vec![1.0; 10]], 3).is_none()); // odd s
    }
}