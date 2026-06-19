//! Multivariate anomaly detector using Mahalanobis distance.
//!
//! Computes the Mahalanobis distance of a feature vector from the rolling
//! distribution's centroid via nalgebra's Cholesky decomposition.

use nalgebra::DVector;
use thiserror::Error;

use crate::stats::RollingFeatureWindow;
use crate::types::FeatureVector;
use crate::FEATURE_DIM;

/// Errors from the Mahalanobis detector.
#[derive(Error, Debug)]
pub enum DetectorError {
    #[error("covariance matrix is not positive-definite (Cholesky failed)")]
    CovarianceNotPositiveDefinite,
    #[error("window is not ready (fewer than minimum bars)")]
    WindowNotReady,
}

/// Mahalanobis distance anomaly detector.
///
/// Computes `d = sqrt((x - μ)^T Σ^{-1} (x - μ))` — the number of standard
/// deviations a point is from the mean, accounting for covariance between
/// features. Uses nalgebra's Cholesky decomposition for stable inversion.
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
    /// Returns `Err` if the window is too small or the covariance fails
    /// Cholesky decomposition (should not happen with regularization).
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

        // Solve Σ y = diff via Cholesky, then d² = diff^T Σ^{-1} diff = diff · y
        let chol = nalgebra::linalg::Cholesky::new(cov)
            .ok_or(DetectorError::CovarianceNotPositiveDefinite)?;
        let y = chol.solve(&diff);
        let d2 = diff.dot(&y);

        Ok(d2.sqrt())
    }

    /// Distance or 0.0 on error (convenience for hot-path code).
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
    fn outlier_is_farther() {
        let mut w = RollingFeatureWindow::new(20);
        fill_window(&mut w, 20);
        let det = MahalanobisDetector::new(2.0, 1e-3);
        let normal = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0];
        let outlier = [5.0, -4.0, 0.8, 0.9, 0.7, 2.0, 50.0, 0.9, 0.8, 10.0];
        assert!(det.distance(&w, &outlier).unwrap() > det.distance(&w, &normal).unwrap());
    }

    #[test]
    fn rejects_small_window() {
        let w = RollingFeatureWindow::new(20);
        let det = MahalanobisDetector::new(2.0, 1e-3);
        let x = [0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0];
        assert!(det.distance(&w, &x).is_err());
    }

    #[test]
    fn known_outlier_distance_is_large() {
        // Build a window with tight, low-variance data so that a moderate
        // outlier produces a large Mahalanobis distance.
        let mut w = RollingFeatureWindow::new(20);
        let baseline = [0.0; FEATURE_DIM];
        for _ in 0..20 {
            w.push(baseline);
        }
        let det = MahalanobisDetector::new(4.0, 0.1);
        // Inject one feature at +10 standard deviations (rough estimate).
        let outlier = [10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let d = det.distance(&w, &outlier).unwrap();
        assert!(d > 1.0, "Mahalanobis distance {d} should be > 1 for clear outlier");
    }

    #[test]
    fn identical_points_have_zero_distance() {
        let mut w = RollingFeatureWindow::new(20);
        let x = [0.5; FEATURE_DIM];
        for _ in 0..20 {
            w.push(x);
        }
        let det = MahalanobisDetector::new(4.0, 1e-4);
        let d = det.distance_or_zero(&w, &x);
        assert!(d < 1.0, "Identical point should have near-zero Mahalanobis distance, got {d}");
    }
}
