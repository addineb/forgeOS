//! Volume profile: POC, value area, and LVN computation from tick data.
//!
//! Builds a volume-at-price histogram from trades, then identifies:
//! - POC (Point of Control): price with highest volume
//! - Value Area: prices containing 70% of total volume around POC
//! - LVNs (Low Volume Nodes): prices with unusually low volume between HVNs

/// A volume profile computed from trade data over a time window.
#[derive(Debug, Clone)]
pub struct VolumeProfile {
    /// Price bins (lower bound of each bin).
    pub bin_prices: Vec<f64>,
    /// Volume in each bin (buy volume).
    pub buy_volumes: Vec<f64>,
    /// Volume in each bin (sell volume).
    pub sell_volumes: Vec<f64>,
    /// Total volume in each bin.
    pub total_volumes: Vec<f64>,
    /// Bin width in price units.
    pub bin_width: f64,
    /// POC (Point of Control): price of the bin with highest total volume.
    pub poc_price: f64,
    /// POC volume.
    pub poc_volume: f64,
    /// Value area high: upper boundary of 70% volume around POC.
    pub value_area_high: f64,
    /// Value area low: lower boundary of 70% volume around POC.
    pub value_area_low: f64,
    /// Total volume across all bins.
    pub total_volume: f64,
}

impl VolumeProfile {
    /// Build a volume profile from a list of (price, qty, side) trades.
    /// `bin_width` is the price width of each bin.
    /// `side` is the aggressor side: Side::Bid = taker bought, Side::Ask = taker sold.
    pub fn from_trades(
        trades: &[(f64, f64, forge_core::Side)],
        bin_width: f64,
    ) -> Self {
        if trades.is_empty() || bin_width <= 0.0 {
            return Self::empty(bin_width);
        }

        // Find price range
        let min_price = trades.iter().map(|(p, _, _)| *p).fold(f64::INFINITY, f64::min);
        let max_price = trades.iter().map(|(p, _, _)| *p).fold(f64::NEG_INFINITY, f64::max);

        let n_bins = ((max_price - min_price) / bin_width).ceil() as usize + 1;
        let mut buy_volumes = vec![0.0_f64; n_bins];
        let mut sell_volumes = vec![0.0_f64; n_bins];
        let mut bin_prices = Vec::with_capacity(n_bins);

        for i in 0..n_bins {
            bin_prices.push(min_price + i as f64 * bin_width);
        }

        // Accumulate trades into bins
        for (price, qty, side) in trades {
            let bin_idx = ((price - min_price) / bin_width).floor() as usize;
            if bin_idx < n_bins {
                match side {
                    forge_core::Side::Bid => buy_volumes[bin_idx] += qty,
                    forge_core::Side::Ask => sell_volumes[bin_idx] += qty,
                }
            }
        }

        let total_volumes: Vec<f64> = buy_volumes.iter()
            .zip(sell_volumes.iter())
            .map(|(b, s)| b + s)
            .collect();
        let total_volume: f64 = total_volumes.iter().sum();

        // Find POC
        let (poc_idx, &poc_volume) = total_volumes.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((0, &0.0));
        let poc_price = bin_prices[poc_idx];

        // Compute value area (70% of volume around POC)
        let va_target = total_volume * 0.70;
        let (va_low, va_high) = if total_volume > 0.0 {
            let mut va_vol = poc_volume;
            let mut low_idx = poc_idx;
            let mut high_idx = poc_idx;

            while va_vol < va_target && (low_idx > 0 || high_idx < n_bins - 1) {
                let expand_low = if low_idx > 0 { total_volumes[low_idx - 1] } else { 0.0 };
                let expand_high = if high_idx < n_bins - 1 { total_volumes[high_idx + 1] } else { 0.0 };

                if expand_low >= expand_high && low_idx > 0 {
                    low_idx -= 1;
                    va_vol += total_volumes[low_idx];
                } else if high_idx < n_bins - 1 {
                    high_idx += 1;
                    va_vol += total_volumes[high_idx];
                } else {
                    low_idx -= 1;
                    va_vol += total_volumes[low_idx];
                }
            }
            (bin_prices[low_idx], bin_prices[high_idx] + bin_width)
        } else {
            (poc_price, poc_price)
        };

        Self {
            bin_prices,
            buy_volumes,
            sell_volumes,
            total_volumes,
            bin_width,
            poc_price,
            poc_volume,
            value_area_high: va_high,
            value_area_low: va_low,
            total_volume,
        }
    }

    /// Find Low Volume Nodes (LVNs): bins with volume < threshold fraction of POC volume.
    /// Returns (price, volume) pairs for each LVN.
    pub fn lvns(&self, threshold: f64) -> Vec<(f64, f64)> {
        let lvn_threshold = self.poc_volume * threshold;
        self.bin_prices.iter()
            .zip(self.total_volumes.iter())
            .filter(|(_, &vol)| vol > 0.0 && vol < lvn_threshold)
            .map(|(&price, &vol)| (price, vol))
            .collect()
    }

    /// Volume concentration: entropy of the volume distribution.
    /// Low entropy = concentrated at POC (strong level).
    /// High entropy = spread evenly (no clear level).
    pub fn concentration(&self) -> f64 {
        if self.total_volume <= 0.0 { return 0.0; }
        let n = self.total_volumes.len() as f64;
        let entropy: f64 = self.total_volumes.iter()
            .filter(|&&v| v > 0.0)
            .map(|&v| {
                let p = v / self.total_volume;
                -p * p.log2()
            })
            .sum();
        // Normalize: max entropy = log2(n_bins), concentration = 1 - entropy/max
        let max_entropy = n.log2();
        if max_entropy > 0.0 { 1.0 - entropy / max_entropy } else { 0.0 }
    }

    fn empty(bin_width: f64) -> Self {
        Self {
            bin_prices: vec![],
            buy_volumes: vec![],
            sell_volumes: vec![],
            total_volumes: vec![],
            bin_width,
            poc_price: 0.0,
            poc_volume: 0.0,
            value_area_high: 0.0,
            value_area_low: 0.0,
            total_volume: 0.0,
        }
    }
}