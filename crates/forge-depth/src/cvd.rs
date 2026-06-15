//! CVD (Cumulative Volume Delta): aggressive buy vs sell volume over time.
//!
//! CVD tracks the net aggressive flow: each trade is classified as buyer-initiated
//! or seller-initiated (using the isBuyerMaker flag: buyer-maker = sell aggression,
//! buyer-taker = buy aggression). Accumulated over rolling windows for macrostructure
//! analysis.

use forge_core::Side;

/// Rolling CVD accumulator with multiple timeframes.
#[derive(Debug, Clone)]
pub struct CVD {
    /// Cumulative buy volume (aggressive taker buys).
    pub buy_vol: f64,
    /// Cumulative sell volume (aggressive taker sells).
    pub sell_vol: f64,
    /// Number of buy trades.
    pub buy_count: u64,
    /// Number of sell trades.
    pub sell_count: u64,
}

impl CVD {
    /// Create a new CVD accumulator at zero.
    pub fn new() -> Self {
        Self {
            buy_vol: 0.0,
            sell_vol: 0.0,
            buy_count: 0,
            sell_count: 0,
        }
    }

    /// Record a trade. `side` is the aggressor side (Side::Bid = taker bought,
    /// Side::Ask = taker sold). `qty` is the trade size.
    pub fn record_trade(&mut self, side: Side, qty: f64) {
        match side {
            Side::Bid => {
                self.buy_vol += qty;
                self.buy_count += 1;
            }
            Side::Ask => {
                self.sell_vol += qty;
                self.sell_count += 1;
            }
        }
    }

    /// Net CVD: buy_vol - sell_vol. Positive = net buying pressure.
    pub fn delta(&self) -> f64 {
        self.buy_vol - self.sell_vol
    }

    /// CVD ratio: buy_vol / (buy_vol + sell_vol). Range [0, 1].
    /// 0.5 = balanced, >0.5 = buy pressure, <0.5 = sell pressure.
    pub fn ratio(&self) -> f64 {
        let total = self.buy_vol + self.sell_vol;
        if total > 0.0 { self.buy_vol / total } else { 0.5 }
    }

    /// Total volume (buy + sell).
    pub fn total_vol(&self) -> f64 {
        self.buy_vol + self.sell_vol
    }

    /// Trade count imbalance: (buy_count - sell_count) / (buy_count + sell_count).
    pub fn count_imbalance(&self) -> f64 {
        let total = self.buy_count + self.sell_count;
        if total > 0 {
            (self.buy_count as f64 - self.sell_count as f64) / total as f64
        } else {
            0.0
        }
    }

    /// Reset accumulator to zero.
    pub fn reset(&mut self) {
        self.buy_vol = 0.0;
        self.sell_vol = 0.0;
        self.buy_count = 0;
        self.sell_count = 0;
    }
}

impl Default for CVD {
    fn default() -> Self {
        Self::new()
    }
}

/// Multi-window CVD tracker. Maintains CVD at several rolling timeframes
/// simultaneously (e.g., 1min, 5min, 15min, 1hr).
/// This is a placeholder for the full ring-buffer implementation.
#[derive(Debug, Clone)]
pub struct MultiWindowCVD {
    /// Window sizes in nanoseconds.
    pub windows_ns: Vec<u64>,
    /// CVD accumulators, one per window.
    pub cvds: Vec<CVD>,
}

impl MultiWindowCVD {
    /// Create a new multi-window CVD tracker with the given window sizes (in seconds).
    pub fn from_seconds(windows_sec: &[u64]) -> Self {
        Self {
            windows_ns: windows_sec.iter().map(|&s| s * 1_000_000_000).collect(),
            cvds: windows_sec.iter().map(|_| CVD::new()).collect(),
        }
    }

    /// Record a trade across all windows.
    pub fn record_trade(&mut self, side: Side, qty: f64) {
        for cvd in &mut self.cvds {
            cvd.record_trade(side, qty);
        }
    }

    /// Get CVD delta for a specific window index.
    pub fn delta(&self, window_idx: usize) -> f64 {
        self.cvds.get(window_idx).map(|c| c.delta()).unwrap_or(0.0)
    }

    /// Get CVD ratio for a specific window index.
    pub fn ratio(&self, window_idx: usize) -> f64 {
        self.cvds.get(window_idx).map(|c| c.ratio()).unwrap_or(0.5)
    }
}