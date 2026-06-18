//! Triple-barrier exit: trade ends when price hits TP, SL, or time expires,
//! whichever comes first. This is the mlfinlab methodology (Ch 3, Snippet 3.2).
//!
//! For each entry bar, we walk forward through subsequent bars checking:
//! - Upper barrier (TP): price moved +tp_bps from entry
//! - Lower barrier (SL): price moved -sl_bps from entry
//! - Vertical barrier (time): hold_bars elapsed
//!
//! The first barrier hit determines the exit and the trade outcome.

/// Which barrier was hit first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarrierHit {
    TakeProfit,
    StopLoss,
    TimeExpired,
}

/// Result of the triple-barrier search.
pub struct TripleBarrier {
    pub exit_idx: usize,
    pub exit_price: f64,
    pub barrier_hit: BarrierHit,
}

impl TripleBarrier {
    /// Walk forward from entry bar `start_idx` and find which barrier is hit first.
    ///
    /// - `tp_bps`: take-profit in bps (0 = disabled)
    /// - `sl_bps`: stop-loss in bps (0 = disabled)
    /// - `hold_bars`: max bars to hold (vertical barrier)
    ///
    /// For a long: TP = price went UP by tp_bps, SL = price went DOWN by sl_bps.
    /// For a short: TP = price went DOWN by tp_bps, SL = price went UP by sl_bps.
    pub fn find(
        bars: &[crate::Bar],
        start_idx: usize,
        entry_price: f64,
        is_long: bool,
        tp_bps: f64,
        sl_bps: f64,
        hold_bars: usize,
    ) -> Self {
        let end_idx = (start_idx + hold_bars).min(bars.len() - 1);
        let tp_threshold = entry_price * (tp_bps / 10000.0);
        let sl_threshold = entry_price * (sl_bps / 10000.0);

        for (j, bar) in bars.iter().enumerate().take(end_idx + 1).skip(start_idx + 1) {
            // Use best_ask as bar high, best_bid as bar low
            // This gives us intra-bar range for more realistic barrier hits
            let high = bar.best_ask;
            let low = bar.best_bid;

            // Check barriers using high/low of the bar
            // (in a full implementation we'd use actual high/low from the bar)
            if is_long {
                // Long: TP when price goes up, SL when price goes down
                if tp_bps > 0.0 && high >= entry_price + tp_threshold {
                    return Self {
                        exit_idx: j,
                        exit_price: entry_price + tp_threshold, // fill at TP price
                        barrier_hit: BarrierHit::TakeProfit,
                    };
                }
                if sl_bps > 0.0 && low <= entry_price - sl_threshold {
                    return Self {
                        exit_idx: j,
                        exit_price: entry_price - sl_threshold, // fill at SL price
                        barrier_hit: BarrierHit::StopLoss,
                    };
                }
            } else {
                // Short: TP when price goes down, SL when price goes up
                if tp_bps > 0.0 && low <= entry_price - tp_threshold {
                    return Self {
                        exit_idx: j,
                        exit_price: entry_price - tp_threshold,
                        barrier_hit: BarrierHit::TakeProfit,
                    };
                }
                if sl_bps > 0.0 && high >= entry_price + sl_threshold {
                    return Self {
                        exit_idx: j,
                        exit_price: entry_price + sl_threshold,
                        barrier_hit: BarrierHit::StopLoss,
                    };
                }
            }
        }

        // Vertical barrier (time expired) — exit at the last bar's mid price
        Self {
            exit_idx: end_idx,
            exit_price: bars[end_idx].mid_price,
            barrier_hit: BarrierHit::TimeExpired,
        }
    }
}
