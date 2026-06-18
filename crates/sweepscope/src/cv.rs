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
/// Supports two modes:
/// - Bar-count mode: splits by equal bar counts (original)
/// - Date-aware mode: splits by trading day boundaries (new)
#[allow(dead_code)]
pub struct PurgedCv {
    /// Total number of bars
    n_bars: usize,
    /// Number of folds
    n_folds: usize,
    /// Purge gap in bars
    purge_bars: usize,
    /// Date boundaries: bar indices where each date starts (including 0 and n_bars)
    date_boundaries: Vec<usize>,
    /// Whether to use date-aware splitting
    date_aware: bool,
}

impl PurgedCv {
    /// Create purged CV splits (bar-count mode).
    ///
    /// - `n_bars`: total number of bars in the dataset
    /// - `n_folds`: number of CV folds
    /// - `purge_bars`: number of bars to exclude between train/test
    pub fn new(n_bars: usize, n_folds: usize, purge_bars: usize) -> Self {
        Self {
            n_bars,
            n_folds,
            purge_bars,
            date_boundaries: vec![0, n_bars],
            date_aware: false,
        }
    }

    /// Create purged CV splits with date-aware splitting.
    ///
    /// - `date_boundaries`: bar indices where each date starts (first date at 0, last at n_bars)
    /// - `n_folds`: number of CV folds
    /// - `purge_bars`: number of bars to exclude between train/test
    pub fn new_date_aware(date_boundaries: Vec<usize>, n_folds: usize, purge_bars: usize) -> Self {
        Self {
            n_bars: *date_boundaries.last().unwrap_or(&0),
            n_folds,
            purge_bars,
            date_boundaries,
            date_aware: true,
        }
    }

    /// Number of folds.
    pub fn n_folds(&self) -> usize {
        self.n_folds
    }

    /// Get the OOS (out-of-sample) range for fold `fold_idx`.
    /// Returns (start, end) as bar indices.
    pub fn oos_range(&self, fold_idx: usize) -> std::ops::Range<usize> {
        if !self.date_aware {
            // Original bar-count mode
            let fold_size = self.n_bars / self.n_folds;
            let start = fold_idx * fold_size;
            let end = if fold_idx == self.n_folds - 1 {
                self.n_bars
            } else {
                (fold_idx + 1) * fold_size
            };
            return start..end;
        }

        // Date-aware mode: split dates into folds
        let n_dates = self.date_boundaries.len() - 1;
        let dates_per_fold = n_dates / self.n_folds;
        let fold_start_date = fold_idx * dates_per_fold;
        let fold_end_date = if fold_idx == self.n_folds - 1 {
            n_dates
        } else {
            (fold_idx + 1) * dates_per_fold
        };
        let start = self.date_boundaries[fold_start_date];
        let end = self.date_boundaries[fold_end_date];
        start..end
    }

    /// Get the IS (in-sample) ranges for fold `fold_idx`.
    /// Returns all bar indices NOT in the OOS range and NOT in the purge zone.
    pub fn is_ranges(&self, fold_idx: usize) -> Vec<std::ops::Range<usize>> {
        let oos = self.oos_range(fold_idx);
        let mut ranges = Vec::new();

        if !self.date_aware {
            // Original bar-count mode
            // Before OOS (with purge gap)
            if oos.start > self.purge_bars {
                ranges.push(0..(oos.start - self.purge_bars));
            }

            // After OOS (with purge gap)
            let after_start = (oos.end + self.purge_bars).min(self.n_bars);
            if after_start < self.n_bars {
                ranges.push(after_start..self.n_bars);
            }
            return ranges;
        }

        // Date-aware mode: find date boundaries for purge
        // Find which dates contain oos.start and oos.end
        let oos_start_date_idx = self.date_boundaries
            .iter()
            .position(|&b| b > oos.start)
            .unwrap_or(self.date_boundaries.len() - 1)
            .saturating_sub(1);
        let oos_end_date_idx = self.date_boundaries
            .iter()
            .position(|&b| b >= oos.end)
            .unwrap_or(self.date_boundaries.len() - 1);

        // Before OOS (with purge gap)
        let purge_before = oos_start_date_idx.saturating_sub(1);
        if purge_before > 0 {
            let is_end = self.date_boundaries[purge_before];
            if is_end > self.purge_bars {
                ranges.push(0..(is_end - self.purge_bars));
            } else {
                ranges.push(0..is_end);
            }
        }

        // After OOS (with purge gap)
        let purge_after = (oos_end_date_idx + 1).min(self.date_boundaries.len() - 1);
        if purge_after < self.date_boundaries.len() - 1 {
            let is_start = self.date_boundaries[purge_after];
            if is_start + self.purge_bars < self.n_bars {
                ranges.push((is_start + self.purge_bars)..self.n_bars);
            }
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
