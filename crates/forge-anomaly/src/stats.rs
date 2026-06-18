//! Rolling window statistics for multivariate feature vectors.

use std::collections::VecDeque;

use ndarray::{Array1, Array2};
#[allow(unused_imports)]
use ndarray_stats::SummaryStatisticsExt;

use crate::types::FeatureVector;
use crate::FEATURE_DIM;

/// Rolling buffer of feature vectors with online mean / covariance.
#[derive(Debug)]
pub struct RollingFeatureWindow {
    window: usize,
    buffer: VecDeque<FeatureVector>,
    feature_sums: [f64; FEATURE_DIM],
    feature_sq_sums: [f64; FEATURE_DIM],
    push_count: u64,
    recompute_interval: u64,
}

impl RollingFeatureWindow {
    #[must_use]
    pub fn new(window: usize) -> Self {
        let w = window.max(4);
        Self {
            window: w,
            buffer: VecDeque::new(),
            feature_sums: [0.0; FEATURE_DIM],
            feature_sq_sums: [0.0; FEATURE_DIM],
            push_count: 0,
            recompute_interval: (w as u64) * 10,
        }
    }

    pub fn push(&mut self, v: FeatureVector) {
        self.push_count += 1;
        if self.buffer.len() >= self.window {
            if let Some(old) = self.buffer.pop_front() {
                for (i, &x) in old.iter().enumerate() {
                    self.feature_sums[i] -= x;
                    self.feature_sq_sums[i] -= x * x;
                }
            }
        }
        for (i, &x) in v.iter().enumerate() {
            self.feature_sums[i] += x;
            self.feature_sq_sums[i] += x * x;
        }
        self.buffer.push_back(v);
        if self.push_count.is_multiple_of(self.recompute_interval) && !self.buffer.is_empty() {
            self.recompute_sums();
        }
    }

    /// Recompute running sums from the buffer to bound floating-point drift.
    fn recompute_sums(&mut self) {
        for i in 0..FEATURE_DIM {
            self.feature_sums[i] = 0.0;
            self.feature_sq_sums[i] = 0.0;
        }
        for v in &self.buffer {
            for (i, &x) in v.iter().enumerate() {
                self.feature_sums[i] += x;
                self.feature_sq_sums[i] += x * x;
            }
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    #[must_use]
    pub fn ready(&self) -> bool {
        self.buffer.len() >= self.window
    }

    /// Population mean vector.
    #[must_use]
    pub fn mean(&self) -> Array1<f64> {
        let n = self.buffer.len() as f64;
        let mut m = Array1::zeros(FEATURE_DIM);
        if n <= 0.0 {
            return m;
        }
        for (i, s) in self.feature_sums.iter().enumerate() {
            m[i] = s / n;
        }
        m
    }

    /// Sample covariance matrix with diagonal regularization.
    #[must_use]
    pub fn covariance(&self, regularization: f64) -> Array2<f64> {
        let n = self.buffer.len();
        let mut cov = Array2::zeros((FEATURE_DIM, FEATURE_DIM));
        if n < 2 {
            for i in 0..FEATURE_DIM {
                cov[[i, i]] = regularization.max(1e-6);
            }
            return cov;
        }
        let mu = self.mean();
        for v in &self.buffer {
            let diff = Array1::from_iter(v.iter().copied()) - &mu;
            for i in 0..FEATURE_DIM {
                for j in 0..FEATURE_DIM {
                    cov[[i, j]] += diff[i] * diff[j];
                }
            }
        }
        let nf = (n - 1) as f64;
        cov.mapv_inplace(|x| x / nf);
        for i in 0..FEATURE_DIM {
            cov[[i, i]] += regularization;
        }
        cov
    }

    /// Per-feature z-score using rolling mean/std (no lookahead on current value
    /// if called before push).
    #[must_use]
    pub fn z_scores(&self, v: &FeatureVector) -> [f64; FEATURE_DIM] {
        let n = self.buffer.len();
        let mut z = [0.0; FEATURE_DIM];
        if n < 2 {
            return z;
        }
        let nf = n as f64;
        for i in 0..FEATURE_DIM {
            let m = self.feature_sums[i] / nf;
            let var = (self.feature_sq_sums[i] / nf - m * m).max(0.0);
            let s = var.sqrt();
            z[i] = if s > 1e-12 { (v[i] - m) / s } else { 0.0 };
        }
        z
    }

    /// Per-feature z-scores with Benjamini-Hochberg FDR correction at level `alpha`.
    /// Returns (z_score, significant) pairs after BH step-up procedure.
    #[must_use]
    pub fn z_scores_fdr(&self, v: &FeatureVector, alpha: f64) -> [(f64, bool); FEATURE_DIM] {
        let z = self.z_scores(v);
        let mut indexed: Vec<(usize, f64, f64)> = z.iter().enumerate().map(|(i, &zv)| {
            let p = two_sided_z_to_p(zv);
            (i, zv, p)
        }).collect();
        indexed.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        let m = FEATURE_DIM as f64;
        let mut significant = [false; FEATURE_DIM];
        let mut last_rank = None;
            for (rank, (_idx, _, p)) in indexed.iter().enumerate() {
            let bh_critical = ((rank + 1) as f64 / m) * alpha;
            if *p <= bh_critical {
                last_rank = Some(rank);
            }
        }
        if let Some(max_rank) = last_rank {
            for (rank, (idx, _, _)) in indexed.iter().enumerate() {
                if rank <= max_rank {
                    significant[*idx] = true;
                }
            }
        }
        let mut out = [(0.0, false); FEATURE_DIM];
        for (i, &zv) in z.iter().enumerate() {
            out[i] = (zv, significant[i]);
        }
        out
    }

    /// All stored vectors as a matrix (rows = samples).
    #[must_use]
    pub fn as_matrix(&self) -> Array2<f64> {
        let n = self.buffer.len();
        let mut m = Array2::zeros((n, FEATURE_DIM));
        for (row, v) in self.buffer.iter().enumerate() {
            for (col, &x) in v.iter().enumerate() {
                m[[row, col]] = x;
            }
        }
        m
    }

    /// Median absolute deviation per feature (robust scale).
    #[must_use]
    pub fn feature_mads(&self) -> [f64; FEATURE_DIM] {
        let mut mads = [0.0; FEATURE_DIM];
        if self.buffer.is_empty() {
            return mads;
        }
        for i in 0..FEATURE_DIM {
            let mut vals: Vec<f64> = self.buffer.iter().map(|v| v[i]).collect();
            let med = median(&vals);
            for v in &mut vals {
                *v = (*v - med).abs();
            }
            mads[i] = median(&vals).max(1e-6);
        }
        mads
    }
}

fn median(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut s = vals.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = s.len() / 2;
    if s.len().is_multiple_of(2) {
        (s[m - 1] + s[m]) / 2.0
    } else {
        s[m]
    }
}

fn two_sided_z_to_p(z: f64) -> f64 {
    let a = z.abs();
    let p = 1.0 / (1.0 + 0.2316419 * a);
    let d = 0.3989422804014327; // 1/sqrt(2*pi)
    let n = p * (1.330274429 - p * (1.821255978 - p * (1.781477937 - p * (0.356563782 - p * 0.319381530))));
    2.0 * d * (-0.5 * a * a).exp() * n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_window_tracks_mean() {
        let mut w = RollingFeatureWindow::new(5);
        for i in 0..5 {
            let mut v = [0.0; FEATURE_DIM];
            v[0] = i as f64;
            w.push(v);
        }
        assert!(w.ready());
        assert!((w.mean()[0] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn ndarray_stats_mean_matches() {
        let mut w = RollingFeatureWindow::new(5);
        for i in 0..5 {
            let mut v = [0.0; FEATURE_DIM];
            v[0] = i as f64;
            w.push(v);
        }
        let col: Vec<f64> = w.buffer.iter().map(|v| v[0]).collect();
        let arr = Array1::from(col);
        let stat_mean = arr.mean().expect("non-empty column");
        assert!((stat_mean - w.mean()[0]).abs() < 1e-9);
    }
}