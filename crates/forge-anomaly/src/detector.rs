//! Multivariate anomaly detectors.
//!
//! - **Mahalanobis distance** (primary): rolling centroid + covariance via nalgebra.
//! - **Isolation Forest** (alternative): stub — work in progress.

use nalgebra::{DMatrix, DVector};
use thiserror::Error;

use crate::stats::RollingFeatureWindow;
use crate::types::{DetectionMethod, FeatureVector};
use crate::FEATURE_DIM;

// ─── Error type ──────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum DetectorError {
    #[error("covariance matrix is not positive-definite (Cholesky failed)")]
    CovarianceNotPositiveDefinite,
    #[error("window is not ready (fewer than minimum bars)")]
    WindowNotReady,
}

// ─── Mahalanobis ─────────────────────────────────────────────────────────────

/// Mahalanobis distance anomaly detector (primary method).
///
/// Computes the Mahalanobis distance of a feature vector from the rolling
/// distribution's centroid, using nalgebra's Cholesky decomposition.
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

    /// Mahalanobis distance of `x` from the rolling distribution.
    ///
    /// Returns `Ok(dist)` when the window is ready and the covariance matrix is
    /// positive-definite. Returns `Err(DetectorError)` otherwise.
    pub fn distance(
        &self,
        window: &RollingFeatureWindow,
        x: &FeatureVector,
    ) -> Result<f64, DetectorError> {
        if !window.ready() {
            return Err(DetectorError::WindowNotReady);
        }

        let mu = window.mean();
        let cov = window.covariance_matrix(self.regularization);
        let diff = DVector::from_iterator(FEATURE_DIM, x.iter().copied()) - mu;

        let d2 = mahalanobis_squared(&diff, &cov)?;
        Ok(d2.sqrt())
    }

    /// Convenience: distance or 0.0 on error.
    #[must_use]
    pub fn distance_or_zero(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> f64 {
        self.distance(window, x).unwrap_or(0.0)
    }

    #[must_use]
    pub fn is_anomaly(&self, window: &RollingFeatureWindow, x: &FeatureVector) -> bool {
        self.distance_or_zero(window, x) >= self.threshold
    }

    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }
}

/// Compute Mahalanobis squared distance using nalgebra's Cholesky decomposition.
fn mahalanobis_squared(
    diff: &DVector<f64>,
    cov: &DMatrix<f64>,
) -> Result<f64, DetectorError> {
    let chol = nalgebra::linalg::Cholesky::new(cov.clone())
        .ok_or(DetectorError::CovarianceNotPositiveDefinite)?;

    solvelnchowithcholesky(&chol, diff)
}

fn solvelnchowithcholesky(
    chol: &nalgebra::linalg::Cholesky<f64, nalgebra::Dyn>,
    diff: &DVector<f64>,
) -> Result<f64, DetectorError> {
    let y = chol.solve(diff);
    Ok(diff.dot(&y))
}

// ─── Isolation Forest (stub) ─────────────────────────────────────────────────

/// Isolation Forest anomaly detector.
///
/// **Work in progress.** Currently returns 0.0 for all inputs. The hand-rolled
/// tree ensemble has known numerical issues; a production implementation should
/// use a dedicated crate or complete the custom implementation.
#[derive(Debug)]
pub struct IsolationForestDetector {
    _n_trees: usize,    threshold: f64,
    epoch: u64,
}

impl IsolationForestDetector {
    #[must_use]
    pub fn new(n_trees: usize, threshold: f64) -> Self {
        Self {
            _n_trees: n_trees.max(8),            threshold: threshold.clamp(0.0, 1.0),
            epoch: 0,
        }
    }

    /// Stub: always returns 0.0 until the Isolation Forest is fully implemented.
    #[must_use]
    pub fn score(&self, _window: &RollingFeatureWindow, _x: &FeatureVector) -> f64 {
        0.0
    }

    #[must_use]
    pub fn is_anomaly(&self, _window: &RollingFeatureWindow, _x: &FeatureVector) -> bool {
        false
    }

    /// Convenience: always returns 0.0 (stub).
    #[must_use]
    pub fn score_cached(&mut self, window: &RollingFeatureWindow, x: &FeatureVector) -> f64 {
        self.score(window, x)
    }

    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    pub fn bump_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }
}

// ─── Combined detector facade ────────────────────────────────────────────────

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

    /// Score the observation; returns `(mahalanobis_dist, isolation_score)`.
    pub fn score(
        &self,
        window: &RollingFeatureWindow,
        x: &FeatureVector,
    ) -> (f64, f64) {
        let m = self.maha.distance_or_zero(window, x);
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
        let normal = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0];
        let outlier = [5.0, -4.0, 0.8, 0.9, 0.7, 2.0, 50.0, 0.9, 0.8, 10.0];
        let d_normal = det.distance(&w, &normal).unwrap();
        let d_outlier = det.distance(&w, &outlier).unwrap();
        assert!(d_outlier > d_normal);
    }

    #[test]
    fn mahalanobis_rejects_tiny_window() {
        let w = RollingFeatureWindow::new(20);
        let det = MahalanobisDetector::new(2.0, 1e-3);
        let x = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0];
        assert!(det.distance(&w, &x).is_err());
    }

    #[test]
    fn isolation_stub_returns_zero() {
        let mut w = RollingFeatureWindow::new(30);
        fill_window(&mut w, 30);
        let det = IsolationForestDetector::new(32, 0.5);
        let x = [5.0, -3.0, 0.5, 0.4, 0.3, 1.0, 20.0, 0.7, 0.6, 5.0];
        assert!((det.score(&w, &x) - 0.0).abs() < 1e-12);
        assert!(!det.is_anomaly(&w, &x));
    }

    #[test]
    fn facade_both_requires_dual_hit() {
        let mut w = RollingFeatureWindow::new(30);
        fill_window(&mut w, 30);
        let det = AnomalyDetector::new(DetectionMethod::Both, 1000.0, 1e-3, 32, 0.99);
        let normal = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0];
        assert!(!det.is_anomaly(&w, &normal));
    }
}
