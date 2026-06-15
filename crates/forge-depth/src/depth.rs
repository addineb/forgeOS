//! Depth snapshot: full L2 book state at a point in time.
//!
//! Unlike the tick-level studies that only used top-5 microprice + imbalance,
//! this captures the full depth shape for macrostructure analysis.

use forge_book::OrderBook;
use forge_core::{Price, Qty};

/// Full depth snapshot computed from an OrderBook at a point in time.
/// Captures shape, concentration, and distribution across all visible levels.
#[derive(Debug, Clone)]
pub struct DepthSnapshot {
    /// Timestamp (nanoseconds).
    pub ts: u64,
    /// Best bid price.
    pub best_bid: f64,
    /// Best ask price.
    pub best_ask: f64,
    /// Spread in bps: (ask - bid) / mid * 10000.
    pub spread_bps: f64,
    /// Mid price: (best_bid + best_ask) / 2.
    pub mid: f64,
    /// Number of bid levels visible.
    pub bid_levels: usize,
    /// Number of ask levels visible.
    pub ask_levels: usize,
    /// Total bid volume across all levels.
    pub total_bid_vol: f64,
    /// Total ask volume across all levels.
    pub total_ask_vol: f64,
    /// Depth imbalance across all levels: (bid - ask) / (bid + ask).
    /// Range [-1, 1]. Positive = bid-heavy (book leans up).
    pub full_imbalance: f64,
    /// Top-N imbalance (configurable N, default 5).
    pub top_n_imbalance: f64,
    /// Volume-weighted mid (microprice across all levels).
    pub vwap_mid: f64,
    /// Bid volume at each level (index 0 = best bid).
    pub bid_volumes: Vec<f64>,
    /// Ask volume at each level (index 0 = best ask).
    pub ask_volumes: Vec<f64>,
    /// Bid price at each level (index 0 = best bid).
    pub bid_prices: Vec<f64>,
    /// Ask price at each level (index 0 = best ask).
    pub ask_prices: Vec<f64>,
}

impl DepthSnapshot {
    /// Compute a depth snapshot from an OrderBook at a given timestamp.
    /// `top_n` controls the imbalance calculation (default 5).
    pub fn from_book(book: &OrderBook, ts: u64, top_n: usize) -> Self {
        let (bb, bbq) = match book.best_bid() {
            Some((p, q)) => (p.to_f64(), q.to_f64()),
            None => return Self::empty(ts),
        };
        let (ba, baq) = match book.best_ask() {
            Some((p, q)) => (p.to_f64(), q.to_f64()),
            None => return Self::empty(ts),
        };

        let mid = (bb + ba) / 2.0;
        let spread_bps = if mid > 0.0 { (ba - bb) / mid * 10000.0 } else { 0.0 };

        // Collect all levels
        let bid_levels: Vec<(f64, f64)> = book.bids_iter()
            .rev() // highest first
            .map(|(p, q)| (p.to_f64(), q.to_f64()))
            .collect();
        let ask_levels: Vec<(f64, f64)> = book.asks_iter()
            .map(|(p, q)| (p.to_f64(), q.to_f64()))
            .collect();

        let bid_volumes: Vec<f64> = bid_levels.iter().map(|(_, q)| *q).collect();
        let ask_volumes: Vec<f64> = ask_levels.iter().map(|(_, q)| *q).collect();
        let bid_prices: Vec<f64> = bid_levels.iter().map(|(p, _)| *p).collect();
        let ask_prices: Vec<f64> = ask_levels.iter().map(|(p, _)| *p).collect();

        let total_bid_vol: f64 = bid_volumes.iter().sum();
        let total_ask_vol: f64 = ask_volumes.iter().sum();
        let total_vol = total_bid_vol + total_ask_vol;

        let full_imbalance = if total_vol > 0.0 {
            (total_bid_vol - total_ask_vol) / total_vol
        } else {
            0.0
        };

        // Top-N imbalance
        let top_bid_vol: f64 = bid_volumes.iter().take(top_n).sum();
        let top_ask_vol: f64 = ask_volumes.iter().take(top_n).sum();
        let top_total = top_bid_vol + top_ask_vol;
        let top_n_imbalance = if top_total > 0.0 {
            (top_bid_vol - top_ask_vol) / top_total
        } else {
            0.0
        };

        // Volume-weighted mid across all levels
        let vwap_mid = if total_vol > 0.0 {
            let bid_vwap: f64 = bid_levels.iter().map(|(p, q)| p * q).sum::<f64>() / total_bid_vol;
            let ask_vwap: f64 = ask_levels.iter().map(|(p, q)| p * q).sum::<f64>() / total_ask_vol;
            (bid_vwap * total_ask_vol + ask_vwap * total_bid_vol) / total_vol
        } else {
            mid
        };

        Self {
            ts,
            best_bid: bb,
            best_ask: ba,
            spread_bps,
            mid,
            bid_levels: bid_levels.len(),
            ask_levels: ask_levels.len(),
            total_bid_vol,
            total_ask_vol,
            full_imbalance,
            top_n_imbalance,
            vwap_mid,
            bid_volumes,
            ask_volumes,
            bid_prices,
            ask_prices,
        }
    }

    /// Inter-level gap at depth index `i` (in bps relative to mid).
    /// Gap between level i and level i+1 on the ask side.
    /// Sparse gaps = easy to sweep through; tight gaps = absorption.
    pub fn ask_gap_bps(&self, i: usize) -> Option<f64> {
        if i + 1 < self.ask_prices.len() && self.mid > 0.0 {
            Some((self.ask_prices[i + 1] - self.ask_prices[i]) / self.mid * 10000.0)
        } else {
            None
        }
    }

    /// Inter-level gap at depth index `i` on the bid side (in bps).
    pub fn bid_gap_bps(&self, i: usize) -> Option<f64> {
        if i + 1 < self.bid_prices.len() && self.mid > 0.0 {
            Some((self.bid_prices[i] - self.bid_prices[i + 1]) / self.mid * 10000.0)
        } else {
            None
        }
    }

    /// Volume at depth band [0..n) for asks.
    pub fn ask_vol_at_band(&self, n: usize) -> f64 {
        self.ask_volumes.iter().take(n).sum()
    }

    /// Volume at depth band [0..n) for bids.
    pub fn bid_vol_at_band(&self, n: usize) -> f64 {
        self.bid_volumes.iter().take(n).sum()
    }

    /// Depth-weighted imbalance: near levels weighted more than far.
    /// Uses exponential decay with factor `decay` (0 < decay < 1).
    pub fn weighted_imbalance(&self, decay: f64) -> f64 {
        let mut bid_w = 0.0_f64;
        let mut ask_w = 0.0_f64;
        let mut weight = 1.0_f64;
        for (i, &vol) in self.bid_volumes.iter().enumerate() {
            bid_w += vol * weight;
            weight *= decay;
            if i > 0 && weight < 1e-10 { break; }
        }
        weight = 1.0;
        for (i, &vol) in self.ask_volumes.iter().enumerate() {
            ask_w += vol * weight;
            weight *= decay;
            if i > 0 && weight < 1e-10 { break; }
        }
        let total = bid_w + ask_w;
        if total > 0.0 { (bid_w - ask_w) / total } else { 0.0 }
    }

    fn empty(ts: u64) -> Self {
        Self {
            ts,
            best_bid: 0.0,
            best_ask: 0.0,
            spread_bps: 0.0,
            mid: 0.0,
            bid_levels: 0,
            ask_levels: 0,
            total_bid_vol: 0.0,
            total_ask_vol: 0.0,
            full_imbalance: 0.0,
            top_n_imbalance: 0.0,
            vwap_mid: 0.0,
            bid_volumes: vec![],
            ask_volumes: vec![],
            bid_prices: vec![],
            ask_prices: vec![],
        }
    }
}