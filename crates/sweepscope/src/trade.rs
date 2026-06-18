#![allow(dead_code)]
//! Trade result tracking.

use crate::barrier::BarrierHit;

/// One completed trade.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TradeResult {
    pub entry_ts: i64,
    pub exit_ts: i64,
    pub entry_idx: usize,
    pub exit_idx: usize,
    pub is_long: bool,
    pub entry_price: f64,
    pub exit_price: f64,
    pub gross_pnl_bps: f64,
    pub net_pnl_bps: f64,
    pub barrier_hit: BarrierHit,
}

/// Summary statistics for a set of trades.
#[allow(dead_code)]
pub struct TradeLog {
    pub trades: Vec<TradeResult>,
}

impl TradeLog {
    pub fn new(trades: Vec<TradeResult>) -> Self {
        Self { trades }
    }

    pub fn round_trips(&self) -> usize {
        self.trades.len()
    }

    pub fn win_rate(&self) -> f64 {
        if self.trades.is_empty() { return 0.0; }
        self.trades.iter().filter(|t| t.net_pnl_bps > 0.0).count() as f64 / self.trades.len() as f64
    }

    pub fn avg_net_pnl_bps(&self) -> f64 {
        if self.trades.is_empty() { return 0.0; }
        self.trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / self.trades.len() as f64
    }

    pub fn total_net_pnl_bps(&self) -> f64 {
        self.trades.iter().map(|t| t.net_pnl_bps).sum()
    }

    /// Returns (tp_count, sl_count, time_count)
    pub fn barrier_counts(&self) -> (usize, usize, usize) {
        let tp = self.trades.iter().filter(|t| t.barrier_hit == BarrierHit::TakeProfit).count();
        let sl = self.trades.iter().filter(|t| t.barrier_hit == BarrierHit::StopLoss).count();
        let tm = self.trades.iter().filter(|t| t.barrier_hit == BarrierHit::TimeExpired).count();
        (tp, sl, tm)
    }

    /// Average hold in bars.
    pub fn avg_hold_bars(&self) -> f64 {
        if self.trades.is_empty() { return 0.0; }
        self.trades.iter().map(|t| t.exit_idx.saturating_sub(t.entry_idx)).sum::<usize>() as f64 / self.trades.len() as f64
    }

    /// Max consecutive losses.
    pub fn max_consecutive_losses(&self) -> usize {
        let mut max = 0;
        let mut current = 0;
        for t in &self.trades {
            if t.net_pnl_bps <= 0.0 {
                current += 1;
                max = max.max(current);
            } else {
                current = 0;
            }
        }
        max
    }
}
