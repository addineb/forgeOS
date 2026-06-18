use forge_core::Side;

#[derive(Debug, Clone)]
pub struct TradeTracker {
    large_threshold: f64,
    bar_buy_vol: f64,
    bar_sell_vol: f64,
    bar_buy_count: u64,
    bar_sell_count: u64,
    bar_large_buy_vol: f64,
    bar_large_sell_vol: f64,
    bar_large_buy_count: u64,
    bar_large_sell_count: u64,
    bar_max_trade_size: f64,
    bar_total_vol: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TradeBarStats {
    pub trade_count: u64,
    pub buy_count: u64,
    pub sell_count: u64,
    pub aggressor_ratio: f64,
    pub large_buy_count: u64,
    pub large_sell_count: u64,
    pub large_buy_vol: f64,
    pub large_sell_vol: f64,
    pub large_aggressor_ratio: f64,
    pub max_trade_size: f64,
    pub trade_intensity: f64,
}

impl TradeTracker {
    pub fn new(large_threshold: f64) -> Self {
        Self {
            large_threshold: large_threshold.max(0.0),
            bar_buy_vol: 0.0,
            bar_sell_vol: 0.0,
            bar_buy_count: 0,
            bar_sell_count: 0,
            bar_large_buy_vol: 0.0,
            bar_large_sell_vol: 0.0,
            bar_large_buy_count: 0,
            bar_large_sell_count: 0,
            bar_max_trade_size: 0.0,
            bar_total_vol: 0.0,
        }
    }

    pub fn record_trade(&mut self, side: Side, qty: f64) {
        let is_large = qty >= self.large_threshold;
        match side {
            Side::Bid => {
                self.bar_buy_vol += qty;
                self.bar_buy_count += 1;
                if is_large {
                    self.bar_large_buy_vol += qty;
                    self.bar_large_buy_count += 1;
                }
            }
            Side::Ask => {
                self.bar_sell_vol += qty;
                self.bar_sell_count += 1;
                if is_large {
                    self.bar_large_sell_vol += qty;
                    self.bar_large_sell_count += 1;
                }
            }
        }
        self.bar_max_trade_size = self.bar_max_trade_size.max(qty);
        self.bar_total_vol += qty;
    }

    pub fn snapshot(&self, bar_vol: f64) -> TradeBarStats {
        let total_count = self.bar_buy_count + self.bar_sell_count;
        let total_large = self.bar_large_buy_count + self.bar_large_sell_count;
        TradeBarStats {
            trade_count: total_count,
            buy_count: self.bar_buy_count,
            sell_count: self.bar_sell_count,
            aggressor_ratio: if total_count > 0 {
                self.bar_buy_count as f64 / total_count as f64
            } else {
                0.5
            },
            large_buy_count: self.bar_large_buy_count,
            large_sell_count: self.bar_large_sell_count,
            large_buy_vol: self.bar_large_buy_vol,
            large_sell_vol: self.bar_large_sell_vol,
            large_aggressor_ratio: if total_large > 0 {
                self.bar_large_buy_count as f64 / total_large as f64
            } else {
                0.5
            },
            max_trade_size: self.bar_max_trade_size,
            trade_intensity: if bar_vol > 0.0 {
                let raw = total_count as f64 / bar_vol;
                raw.min(20.0)
            } else { 0.0 },
        }
    }

    pub fn reset_bar(&mut self) {
        self.bar_buy_vol = 0.0;
        self.bar_sell_vol = 0.0;
        self.bar_buy_count = 0;
        self.bar_sell_count = 0;
        self.bar_large_buy_vol = 0.0;
        self.bar_large_sell_vol = 0.0;
        self.bar_large_buy_count = 0;
        self.bar_large_sell_count = 0;
        self.bar_max_trade_size = 0.0;
        self.bar_total_vol = 0.0;
    }
}
