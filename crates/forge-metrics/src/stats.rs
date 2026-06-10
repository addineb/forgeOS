//! Moment statistics and the Sharpe ratio (population moments, divide by n).

/// Mean of a series (0 if empty).
#[must_use]
pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    x.iter().sum::<f64>() / x.len() as f64
}

/// Population standard deviation (divide by n). 0 if fewer than 2 points.
#[must_use]
pub fn std_dev(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    let var = x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / x.len() as f64;
    var.sqrt()
}

fn central_moment(x: &[f64], k: i32) -> f64 {
    let m = mean(x);
    x.iter().map(|v| (v - m).powi(k)).sum::<f64>() / x.len() as f64
}

/// Population skewness (Fisher). 0 if degenerate.
#[must_use]
pub fn skewness(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m2 = central_moment(x, 2);
    if m2 <= 0.0 {
        return 0.0;
    }
    central_moment(x, 3) / m2.powf(1.5)
}

/// Population kurtosis (non-excess; a normal distribution is 3). 0 if degenerate.
#[must_use]
pub fn kurtosis(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m2 = central_moment(x, 2);
    if m2 <= 0.0 {
        return 0.0;
    }
    central_moment(x, 4) / (m2 * m2)
}

/// Sharpe ratio = mean / std (per observation, not annualised). 0 if no spread.
#[must_use]
pub fn sharpe(x: &[f64]) -> f64 {
    let s = std_dev(x);
    if s <= 0.0 {
        return 0.0;
    }
    mean(x) / s
}

/// Summary of a trade-return series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeStats {
    /// Number of observations (e.g. round-trip P&Ls).
    pub n: usize,
    /// Sum of the series (total net).
    pub total: f64,
    /// Mean per observation.
    pub mean: f64,
    /// Population standard deviation.
    pub std: f64,
    /// Sharpe (mean / std).
    pub sharpe: f64,
    /// Fraction of positive observations.
    pub win_rate: f64,
}

impl EdgeStats {
    /// Summarise a series of per-trade returns / P&Ls.
    #[must_use]
    pub fn from_series(x: &[f64]) -> Self {
        let n = x.len();
        let wins = x.iter().filter(|&&v| v > 0.0).count();
        Self {
            n,
            total: x.iter().sum(),
            mean: mean(x),
            std: std_dev(x),
            sharpe: sharpe(x),
            win_rate: if n == 0 { 0.0 } else { wins as f64 / n as f64 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_moments() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((mean(&x) - 3.0).abs() < 1e-12);
        assert!((std_dev(&x) - 2.0_f64.sqrt()).abs() < 1e-9); // pop var = 2
        assert!(skewness(&x).abs() < 1e-9); // symmetric
    }

    #[test]
    fn sharpe_signs() {
        let up = [1.0, 1.1, 0.9, 1.0, 1.2];
        assert!(sharpe(&up) > 0.0);
        let flat = [1.0, 1.0, 1.0];
        assert_eq!(sharpe(&flat), 0.0);
        let down = [-1.0, -1.1, -0.9];
        assert!(sharpe(&down) < 0.0);
    }

    #[test]
    fn edge_stats_win_rate() {
        let x = [1.0, -1.0, 2.0, -0.5];
        let e = EdgeStats::from_series(&x);
        assert_eq!(e.n, 4);
        assert!((e.total - 1.5).abs() < 1e-12);
        assert!((e.win_rate - 0.5).abs() < 1e-12);
    }
}