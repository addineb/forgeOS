#![allow(dead_code)]
//! Purged cross-validation (mlfinlab Ch 7, Snippet 7.1).
//!
//! Standard K-fold CV leaks information when trades overlap across fold
//! boundaries. Purged CV adds a gap (purge) between train and test sets
//! so no trade in the test set could have been influenced by data in the
//! training set.
//!
//! For volume bars, the purge is measured in bars (not time), since each
//! bar represents 10 BTC of trading activity.

/// Purged K-fold cross-validation splits.
#[allow(dead_code)]
pub struct PurgedCv {
    n_bars: usize,
    n_folds: usize,
    purge_bars: usize,
}

impl PurgedCv {
    /// Create purged CV splits.
    ///
    /// - `n_bars`: total number of bars in the dataset
    /// - `n_folds`: number of CV folds
    /// - `purge_bars`: number of bars to exclude between train/test
    pub fn new(n_bars: usize, n_folds: usize, purge_bars: usize) -> Self {
        Self { n_bars, n_folds, purge_bars }
    }

    /// Number of folds.
    pub fn n_folds(&self) -> usize {
        self.n_folds
    }

    /// Get the OOS (out-of-sample) range for fold `fold_idx`.
    /// Returns (start, end) as bar indices.
    pub fn oos_range(&self, fold_idx: usize) -> std::ops::Range<usize> {
        let fold_size = self.n_bars / self.n_folds;
        let start = fold_idx * fold_size;
        let end = if fold_idx == self.n_folds - 1 {
            self.n_bars
        } else {
            (fold_idx + 1) * fold_size
        };
        start..end
    }

    /// Get the IS (in-sample) ranges for fold `fold_idx`.
    /// Returns all bar indices NOT in the OOS range and NOT in the purge zone.
    pub fn is_ranges(&self, fold_idx: usize) -> Vec<std::ops::Range<usize>> {
        let oos = self.oos_range(fold_idx);
        let mut ranges = Vec::new();

        // Before OOS (with purge gap)
        if oos.start > self.purge_bars {
            ranges.push(0..(oos.start - self.purge_bars));
        }

        // After OOS (with purge gap)
        let after_start = (oos.end + self.purge_bars).min(self.n_bars);
        if after_start < self.n_bars {
            ranges.push(after_start..self.n_bars);
        }

        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_purged_cv_splits() {
        let cv = PurgedCv::new(100, 5, 5);
        assert_eq!(cv.n_folds(), 5);

        // Fold 0: OOS = bars 0-19, IS = bars 25-99
        let oos = cv.oos_range(0);
        assert_eq!(oos, 0..20);
        let is_ranges = cv.is_ranges(0);
        assert_eq!(is_ranges.len(), 1);
        assert_eq!(is_ranges[0], 25..100);

        // Fold 2: OOS = bars 40-59, IS = bars 0-35 and 65-99
        let oos = cv.oos_range(2);
        assert_eq!(oos, 40..60);
        let is_ranges = cv.is_ranges(2);
        assert_eq!(is_ranges.len(), 2);
        assert_eq!(is_ranges[0], 0..35);
        assert_eq!(is_ranges[1], 65..100);
    }
}
