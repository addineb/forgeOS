//! Multivariate anomaly detectors.
//!
//! - **Mahalanobis distance** (primary): rolling centroid + covariance
//! - **Isolation Forest** (alternative): random-partition tree ensemble

use linfa::{Dataset, DatasetBase};
use ndarray::{Array1, Array2};

use crate::prng;
use crate::stats::RollingFeatureWindow;
use crate::types::{DetectionMethod, FeatureVector};
use crate::FEATURE_DIM;

// ─── Mahalanobis ─────────────────────────────────────────────────────────────

/// Mahalanobis distance anomaly detector (primary method).
#[derive(Debug)]
pub struct MahalanobisDetector {
    threshold: f64,
    regularization: f64,
}

impl MahalanobisDetector {
    #[must_use]
    pub fn new(threshold: f64, regularization: f64) -> Self {
        Self {
            threshold: threshold.max(0.1),
            regularization: regularization.max(1e-8),
        }
    }

    /// Distance of `x` from the rolling distribution. 0 if not ready.
    #[must_use]
    pub fn distance(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> f64 {
        if !window.ready() {
            return 0.0;
        }
        let mu = window.mean();
        let cov = window.covariance(self.regularization);
        let diff = Array1::from_iter(x.iter().copied()) - mu;
        mahalanobis_squared(&diff, &cov).sqrt()
    }

    #[must_use]
    pub fn is_anomaly(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> bool {
        self.distance(window, x) >= self.threshold
    }

    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }
}

fn mahalanobis_squared(diff: &Array1<f64>, cov: &Array2<f64>) -> f64 {
    let n = diff.len();
    if let Some(l) = cholesky_lower(cov) {
        let y = forward_substitute(&l, diff);
        y.iter().map(|v| v * v).sum()
    } else if let Some(inv) = invert_matrix(cov) {
        let mut d2 = 0.0;
        for i in 0..n {
            for j in 0..n {
                d2 += diff[i] * inv[[i, j]] * diff[j];
            }
        }
        d2.max(0.0)
    } else {
        0.0
    }
}

fn cholesky_lower(a: &Array2<f64>) -> Option<Array2<f64>> {
    let n = a.nrows();
    let mut l = Array2::zeros((n, n));
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[[i, j]];
            for k in 0..j {
                sum -= l[[i, k]] * l[[j, k]];
            }
            if i == j {
                if sum <= 1e-12 {
                    return None;
                }
                l[[i, j]] = sum.sqrt();
            } else {
                l[[i, j]] = sum / l[[j, j]];
            }
        }
    }
    Some(l)
}

fn forward_substitute(l: &Array2<f64>, b: &Array1<f64>) -> Array1<f64> {
    let n = b.len();
    let mut y = Array1::zeros(n);
    for i in 0..n {
        let mut sum = b[i];
        for k in 0..i {
            sum -= l[[i, k]] * y[k];
        }
        y[i] = sum / l[[i, i]];
    }
    y
}

fn invert_matrix(a: &Array2<f64>) -> Option<Array2<f64>> {
    let n = a.nrows();
    let mut aug = Array2::zeros((n, 2 * n));
    for i in 0..n {
        for j in 0..n {
            aug[[i, j]] = a[[i, j]];
        }
        aug[[i, n + i]] = 1.0;
    }
    for col in 0..n {
        let mut pivot = col;
        for row in (col + 1)..n {
            if aug[[row, col]].abs() > aug[[pivot, col]].abs() {
                pivot = row;
            }
        }
        if aug[[pivot, col]].abs() < 1e-12 {
            return None;
        }
        if pivot != col {
            for k in 0..(2 * n) {
                let tmp = aug[[col, k]];
                aug[[col, k]] = aug[[pivot, k]];
                aug[[pivot, k]] = tmp;
            }
        }
        let div = aug[[col, col]];
        for k in 0..(2 * n) {
            aug[[col, k]] /= div;
        }
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = aug[[row, col]];
            if factor.abs() < 1e-15 {
                continue;
            }
            for k in 0..(2 * n) {
                aug[[row, k]] -= factor * aug[[col, k]];
            }
        }
    }
    let mut inv = Array2::zeros((n, n));
    for i in 0..n {
        for j in 0..n {
            inv[[i, j]] = aug[[i, n + j]];
        }
    }
    Some(inv)
}

// ─── Isolation Forest ────────────────────────────────────────────────────────

/// Isolation Forest detector (alternative / advanced method).
#[derive(Debug)]
pub struct IsolationForestDetector {
    n_trees: usize,
    threshold: f64,
    epoch: u64,
    cached_trees: Option<Vec<IsoNode>>,
    cache_epoch: u64,
    max_subsample: usize,
}

impl IsolationForestDetector {
    #[must_use]
    pub fn new(n_trees: usize, threshold: f64) -> Self {
        Self {
            n_trees: n_trees.max(8),
            threshold: threshold.clamp(0.0, 1.0),
            epoch: 0,
            cached_trees: None,
            cache_epoch: u64::MAX,
            max_subsample: 256,
        }
    }

    #[must_use]
    pub fn score(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> f64 {
        if !window.ready() {
            return 0.0;
        }
        let data = window.as_matrix();
        let sample = Array1::from_iter(x.iter().copied());
        let avg_path = self.average_path_length_sub(&data, &sample);
        let c = avg_path_length_normalizer(window.len());
        2.0_f64.powf(-avg_path / c).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn is_anomaly(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> bool {
        self.score(window, x) >= self.threshold
    }

    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    pub fn bump_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    #[must_use]
    pub fn to_dataset(window: &RollingFeatureWindow) -> DatasetBase<Array2<f64>, Array1<()>> {
        Dataset::from(window.as_matrix())
    }

    pub fn score_cached(&mut self, window: &RollingFeatureWindow, x: &FeatureVector) -> f64 {
        if !window.ready() {
            return 0.0;
        }
        let data = window.as_matrix();
        let sample = Array1::from_iter(x.iter().copied());
        if self.cache_epoch != self.epoch {
            self.rebuild_cache(window);
        }
        let avg_path = self.average_path_length_cached(&data, &sample);
        let c = avg_path_length_normalizer(window.len());
        2.0_f64.powf(-avg_path / c).clamp(0.0, 1.0)
    }

    fn average_path_length_sub(&self, data: &Array2<f64>, sample: &Array1<f64>) -> f64 {
        let n_total = data.nrows();
        let subsample = self.max_subsample.min(n_total);
        let base_seed = self.epoch.wrapping_mul(0x5851_F42D_4C95_7F2D);

        let mut total = 0.0;
        for t in 0..self.n_trees {
            let tree_seed = prng::splitmix64(base_seed.wrapping_add(t as u64));
            let mut indices: Vec<usize> = Vec::with_capacity(subsample);
            let mut rng = prng::splitmix64(tree_seed);
            for _ in 0..subsample {
                rng = prng::splitmix64(rng);
                indices.push((rng as usize) % n_total);
            }
                let sub_data = subset_rows(data, &indices);
            let tree = build_tree(&sub_data, 0, subsample, 0, max_depth(subsample), prng::splitmix64(rng));
            total += path_length(&tree, sample, 0);
        }
        total / self.n_trees as f64
    }

    fn rebuild_cache(&mut self, window: &RollingFeatureWindow) {
        if !window.ready() {
            self.cached_trees = None;
            self.cache_epoch = u64::MAX;
            return;
        }
        let data = window.as_matrix();
        let n_total = data.nrows();
        let subsample = self.max_subsample.min(n_total);
        let base_seed = self.epoch.wrapping_mul(0x5851_F42D_4C95_7F2D);

        self.cached_trees = Some((0..self.n_trees).map(|t| {
            let tree_seed = prng::splitmix64(base_seed.wrapping_add(t as u64));
            let mut indices: Vec<usize> = Vec::with_capacity(subsample);
            let mut rng = prng::splitmix64(tree_seed);
            for _ in 0..subsample {
                rng = prng::splitmix64(rng);
                indices.push((rng as usize) % n_total);
            }
                let sub_data = subset_rows(&data, &indices);
            build_tree(&sub_data, 0, subsample, 0, max_depth(subsample), prng::splitmix64(rng))
        }).collect());
        self.cache_epoch = self.epoch;
    }

    fn average_path_length_cached(&self, data: &Array2<f64>, sample: &Array1<f64>) -> f64 {
        if let Some(ref trees) = self.cached_trees {
            let mut total = 0.0;
            for tree in trees {
                total += path_length(tree, sample, 0);
            }
            total / self.n_trees as f64
        } else {
            self.average_path_length_sub(data, sample)
        }
    }
}

#[derive(Debug)]
struct IsoNode {
    split_feature: usize,
    split_value: f64,
    left: Option<Box<IsoNode>>,
    right: Option<Box<IsoNode>>,
    size: usize,
}

fn max_depth(n: usize) -> usize {
    (n as f64).log2().ceil() as usize + 1
}

fn avg_path_length_normalizer(n: usize) -> f64 {
    if n <= 1 {
        return 1.0;
    }
    let nf = n as f64;
    2.0 * ((nf - 1.0).ln() + 0.577_215_664_9) - 2.0 * (nf - 1.0) / nf
}

fn build_tree(data: &Array2<f64>, start: usize, end: usize, depth: usize, max_d: usize, seed: u64) -> IsoNode {
    let size = end - start;
    if size <= 1 || depth >= max_d {
        return IsoNode {
            split_feature: 0,
            split_value: 0.0,
            left: None,
            right: None,
            size,
        };
    }
    let mut rng = seed;
    let feat = (prng::splitmix64(rng) as usize) % FEATURE_DIM;
    rng = prng::splitmix64(rng);

    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    for i in start..end {
        let v = data[[i, feat]];
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }
    if (max_v - min_v).abs() < 1e-12 {
        return IsoNode {
            split_feature: feat,
            split_value: min_v,
            left: None,
            right: None,
            size,
        };
    }
    let split = min_v + (prng::splitmix64(rng) as f64 / u64::MAX as f64) * (max_v - min_v);

    let mut left_idx = Vec::new();
    let mut right_idx = Vec::new();
    for i in start..end {
        if data[[i, feat]] < split {
            left_idx.push(i);
        } else {
            right_idx.push(i);
        }
    }
    if left_idx.is_empty() || right_idx.is_empty() {
        return IsoNode {
            split_feature: feat,
            split_value: split,
            left: None,
            right: None,
            size,
        };
    }

    let left_data = subset_rows(data, &left_idx);
    let right_data = subset_rows(data, &right_idx);

    IsoNode {
        split_feature: feat,
        split_value: split,
        left: Some(Box::new(build_tree(
            &left_data, 0, left_data.nrows(), depth + 1, max_d, prng::splitmix64(rng),
        ))),
        right: Some(Box::new(build_tree(
            &right_data, 0, right_data.nrows(), depth + 1, max_d, prng::splitmix64(rng.wrapping_add(1)),
        ))),
        size,
    }
}

fn subset_rows(data: &Array2<f64>, indices: &[usize]) -> Array2<f64> {
    let mut out = Array2::zeros((indices.len(), FEATURE_DIM));
    for (r, &idx) in indices.iter().enumerate() {
        for c in 0..FEATURE_DIM {
            out[[r, c]] = data[[idx, c]];
        }
    }
    out
}

fn path_length(node: &IsoNode, sample: &Array1<f64>, depth: usize) -> f64 {
    if node.left.is_none() && node.right.is_none() {
        return depth as f64 + avg_path_length_normalizer(node.size);
    }
    if sample[node.split_feature] < node.split_value {
        if let Some(ref left) = node.left {
            path_length(left, sample, depth + 1)
        } else {
            depth as f64 + 1.0
        }
    } else if let Some(ref right) = node.right {
        path_length(right, sample, depth + 1)
    } else {
        depth as f64 + 1.0
    }
}

// ─── Unified facade ──────────────────────────────────────────────────────────

/// Combined detector selecting Mahalanobis, Isolation Forest, or both.
#[derive(Debug)]
pub struct AnomalyDetector {
    pub maha: MahalanobisDetector,
    pub iso: IsolationForestDetector,
    pub method: DetectionMethod,
}

impl AnomalyDetector {
    #[must_use]
    pub fn new(
        method: DetectionMethod,
        maha_threshold: f64,
        cov_regularization: f64,
        iso_trees: usize,
        iso_threshold: f64,
    ) -> Self {
        Self {
            maha: MahalanobisDetector::new(maha_threshold, cov_regularization),
            iso: IsolationForestDetector::new(iso_trees, iso_threshold),
            method,
        }
    }

    /// Score the observation; returns (mahalanobis_dist, isolation_score).
    pub fn score(
        &self,
        window: &RollingFeatureWindow,
        x: &FeatureVector,
    ) -> (f64, f64) {
        let m = self.maha.distance(window, x);
        let i = self.iso.score(window, x);
        (m, i)
    }

    /// Whether the configured method flags this as an anomaly.
    pub fn is_anomaly(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> bool {
        let maha_hit = self.maha.is_anomaly(window, x);
        let iso_hit = self.iso.is_anomaly(window, x);
        match self.method {
            DetectionMethod::Mahalanobis => maha_hit,
            DetectionMethod::IsolationForest => iso_hit,
            DetectionMethod::Both => maha_hit && iso_hit,
            DetectionMethod::Either => maha_hit || iso_hit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_window(w: &mut RollingFeatureWindow, n: usize) {
        for _ in 0..n {
            let mut v = [0.0; FEATURE_DIM];
            v[0] = 0.1;
            v[1] = 0.05;
            w.push(v);
        }
    }

    #[test]
    fn mahalanobis_outlier_farther() {
        let mut w = RollingFeatureWindow::new(20);
        fill_window(&mut w, 20);
        let det = MahalanobisDetector::new(2.0, 1e-3);
        let normal = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0];
        let outlier = [5.0, -4.0, 0.8, 0.9, 0.7, 2.0, 50.0];
        assert!(det.distance(&w, &outlier) > det.distance(&w, &normal));
    }

    #[test]
    fn isolation_outlier_scores_higher() {
        let mut w = RollingFeatureWindow::new(30);
        fill_window(&mut w, 30);
        let det = IsolationForestDetector::new(32, 0.5);
        let normal = [0.0; FEATURE_DIM];
        let outlier = [10.0, -8.0, 0.9, 0.8, 0.7, 5.0, 100.0];
        assert!(det.score(&w, &outlier) >= det.score(&w, &normal));
    }

    #[test]
    fn isolation_score_is_deterministic() {
        let mut w = RollingFeatureWindow::new(30);
        fill_window(&mut w, 30);
        let det = IsolationForestDetector::new(32, 0.5);
        let x = [5.0, -3.0, 0.5, 0.4, 0.3, 1.0, 20.0];
        let s1 = det.score(&w, &x);
        let s2 = det.score(&w, &x);
        assert!((s1 - s2).abs() < 1e-12, "score must be idempotent: {} vs {}", s1, s2);
    }

    #[test]
    fn facade_both_requires_dual_hit() {
        let mut w = RollingFeatureWindow::new(30);
        fill_window(&mut w, 30);
        let det = AnomalyDetector::new(DetectionMethod::Both, 1000.0, 1e-3, 32, 0.99);
        let normal = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert!(!det.is_anomaly(&w, &normal));
    }
}