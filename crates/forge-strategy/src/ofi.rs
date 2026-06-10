//! Order-Flow Imbalance (OFI), re-derived from Cont-Kukanov-Stoikov
//! ("The Price Impact of Order Book Events", arXiv:1011.6402).
//!
//! Per top-of-book update the event contribution is
//! `e = [P_b>=P_b'] q_b - [P_b<=P_b'] q_b' - [P_a<=P_a'] q_a + [P_a>=P_a'] q_a'`
//! (primes = previous), i.e. bid size added or bid price up = buy pressure;
//! ask size added or ask price down = sell pressure. We sum `e` over a rolling
//! window and (optionally) divide by the window-average top-of-book depth to
//! make the signal scale-free / regime-comparable (the depth scaling CKS show
//! the price impact coefficient is inversely proportional to).

use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Top {
    bid_px: i64,
    bid_qty: i64,
    ask_px: i64,
    ask_qty: i64,
}

/// Rolling-window OFI estimator over top-of-book updates.
pub struct Ofi {
    window: usize,
    contribs: VecDeque<i64>,
    sum: i64,
    depths: VecDeque<i64>,
    depth_sum: i64,
    prev: Option<Top>,
}

impl Ofi {
    /// New estimator summing over the last `window` updates (>= 1).
    #[must_use]
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            contribs: VecDeque::new(),
            sum: 0,
            depths: VecDeque::new(),
            depth_sum: 0,
            prev: None,
        }
    }

    /// The CKS per-event contribution between two top-of-book states.
    fn event(p: &Top, c: &Top) -> i64 {
        let mut e = 0i64;
        if c.bid_px >= p.bid_px {
            e += c.bid_qty;
        }
        if c.bid_px <= p.bid_px {
            e -= p.bid_qty;
        }
        if c.ask_px <= p.ask_px {
            e -= c.ask_qty;
        }
        if c.ask_px >= p.ask_px {
            e += p.ask_qty;
        }
        e
    }

    fn push_contrib(&mut self, e: i64) {
        self.contribs.push_back(e);
        self.sum += e;
        if self.contribs.len() > self.window {
            if let Some(old) = self.contribs.pop_front() {
                self.sum -= old;
            }
        }
    }

    fn push_depth(&mut self, d: i64) {
        self.depths.push_back(d);
        self.depth_sum += d;
        if self.depths.len() > self.window {
            if let Some(old) = self.depths.pop_front() {
                self.depth_sum -= old;
            }
        }
    }

    /// Feed the current top of book (raw fixed-point prices and sizes).
    pub fn observe(&mut self, bid_px: i64, bid_qty: i64, ask_px: i64, ask_qty: i64) {
        let cur = Top { bid_px, bid_qty, ask_px, ask_qty };
        if let Some(p) = self.prev {
            let e = Self::event(&p, &cur);
            self.push_contrib(e);
            self.push_depth((bid_qty + ask_qty) / 2);
        }
        self.prev = Some(cur);
    }

    /// True once a full window of contributions has accumulated.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.contribs.len() >= self.window
    }

    /// Raw windowed OFI (fixed-point qty units).
    #[must_use]
    pub fn raw(&self) -> i64 {
        self.sum
    }

    /// Depth-normalised OFI: windowed OFI divided by the window-average
    /// top-of-book depth. Scale-free, regime-comparable. 0 if no depth seen.
    #[must_use]
    pub fn normalized(&self) -> f64 {
        if self.depths.is_empty() {
            return 0.0;
        }
        let avg = self.depth_sum as f64 / self.depths.len() as f64;
        if avg <= 0.0 {
            return 0.0;
        }
        self.sum as f64 / avg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bid_size_added_is_buy_pressure() {
        let mut o = Ofi::new(8);
        o.observe(100, 10, 101, 10); // seed
        o.observe(100, 15, 101, 10); // bid grew by 5 at same price -> +5
        assert_eq!(o.raw(), 5);
    }

    #[test]
    fn ask_size_added_is_sell_pressure() {
        let mut o = Ofi::new(8);
        o.observe(100, 10, 101, 10);
        o.observe(100, 10, 101, 15); // ask grew by 5 -> -5
        assert_eq!(o.raw(), -5);
    }

    #[test]
    fn bid_price_up_is_buy_pressure() {
        let mut o = Ofi::new(8);
        o.observe(100, 10, 101, 10);
        o.observe(101, 8, 102, 10); // bid price up -> +cur.bid_qty (8); ask price up -> +prev.ask_qty (10)
        assert_eq!(o.raw(), 18);
    }

    #[test]
    fn window_rolls_off_old_contributions() {
        let mut o = Ofi::new(2);
        o.observe(100, 10, 101, 10);
        o.observe(100, 12, 101, 10); // +2
        o.observe(100, 15, 101, 10); // +3 (window now [+2,+3]=5)
        o.observe(100, 16, 101, 10); // +1 (rolls off +2 -> [+3,+1]=4)
        assert_eq!(o.raw(), 4);
        assert!(o.ready());
    }

    #[test]
    fn normalized_is_scale_free() {
        let mut o = Ofi::new(4);
        o.observe(100, 100, 101, 100);
        o.observe(100, 150, 101, 100); // +50 with depth ~125
        let n = o.normalized();
        assert!(n > 0.0 && n < 1.0);
    }
}