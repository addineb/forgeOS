#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    Trending,
    Ranging,
    Volatile,
}

impl std::fmt::Display for MarketRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trending => write!(f, "trending"),
            Self::Ranging => write!(f, "ranging"),
            Self::Volatile => write!(f, "volatile"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegimeDetector {
    lookback: usize,
    vol_threshold: f64,
    autocorr_threshold: f64,
}

impl RegimeDetector {
    #[must_use]
    pub fn new(lookback: usize, vol_threshold: f64, autocorr_threshold: f64) -> Self {
        Self {
            lookback: lookback.max(4),
            vol_threshold: vol_threshold.max(1.0),
            autocorr_threshold: autocorr_threshold.clamp(0.0, 1.0),
        }
    }

    #[must_use]
    pub fn classify(&self, returns: &[f64]) -> MarketRegime {
        let n = returns.len().min(self.lookback);
        if n < 2 {
            return MarketRegime::Ranging;
        }
        let recent = &returns[returns.len().saturating_sub(n)..];
        let mean = recent.iter().sum::<f64>() / n as f64;
        let var = recent.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / (n - 1) as f64;
        let std = var.sqrt();
        if std > self.vol_threshold {
            return MarketRegime::Volatile;
        }
        let (mut lag1_num, mut lag1_den) = (0.0, 0.0);
        for i in 1..n {
            lag1_num += (recent[i] - mean) * (recent[i - 1] - mean);
            lag1_den += (recent[i] - mean) * (recent[i] - mean);
        }
        let autocorr = if lag1_den > 1e-12 { lag1_num / lag1_den } else { 0.0 };
        if autocorr > self.autocorr_threshold {
            MarketRegime::Trending
        } else {
            MarketRegime::Ranging
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_on_high_std() {
        let rd = RegimeDetector::new(10, 5.0, 0.2);
        let returns = vec![2.0, -8.0, 3.0, 12.0, -6.0, 4.0, -7.0, 9.0, -3.0, 5.0];
        assert_eq!(rd.classify(&returns), MarketRegime::Volatile);
    }

    #[test]
    fn trending_on_autocorrelation() {
        let rd = RegimeDetector::new(20, 15.0, 0.2);
        let returns: Vec<f64> = (0..20).map(|i| 0.5 + (i as f64) * 0.1).collect();
        assert_eq!(rd.classify(&returns), MarketRegime::Trending);
    }

    #[test]
    fn ranging_on_low_std_and_acorr() {
        let rd = RegimeDetector::new(10, 15.0, 0.2);
        let returns = vec![0.5, -0.3, 0.2, -0.1, 0.4, -0.2, 0.1, -0.4, 0.3, -0.5];
        assert_eq!(rd.classify(&returns), MarketRegime::Ranging);
    }
}
