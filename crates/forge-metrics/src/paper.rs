//! Paper-trade account simulation: the pre-live gate. Take a promoted bot's
//! per-trip returns and run them through a REAL capital model - a starting
//! balance (e.g. EUR 500), leverage, position sizing, and a daily-loss-limit -
//! to see whether it survives and profits the actual money you would risk. This
//! is leverage-AWARE on purpose (unlike the edge metric): it answers "is this
//! tradeable for MY account", including ruin and drawdown.

/// Account parameters for the paper run.
#[derive(Clone, Copy, Debug)]
pub struct PaperConfig {
    /// Starting balance (account currency, e.g. EUR).
    pub start_balance: f64,
    /// Leverage applied to the margin per trade.
    pub leverage: f64,
    /// Fraction of balance committed as margin per trade (e.g. 0.10 = 10%).
    pub risk_pct: f64,
    /// Halt trading for the rest of a day after losing this fraction of the
    /// day's starting balance (0 = disabled).
    pub daily_loss_limit_pct: f64,
}

impl Default for PaperConfig {
    fn default() -> Self {
        Self { start_balance: 500.0, leverage: 20.0, risk_pct: 0.20, daily_loss_limit_pct: 0.05 }
    }
}

/// Outcome of a paper run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaperResult {
    /// Starting balance.
    pub start: f64,
    /// Ending balance.
    pub end: f64,
    /// Total return percent.
    pub return_pct: f64,
    /// Worst peak-to-trough balance drawdown (percent).
    pub max_drawdown_pct: f64,
    /// Lowest balance reached.
    pub min_balance: f64,
    /// Trips actually taken (excludes those skipped by a daily halt).
    pub trips_taken: usize,
    /// Number of days the daily-loss-limit halted trading.
    pub halted_days: usize,
    /// Whether the account hit zero (ruin).
    pub ruined: bool,
}

const DAY_NS: u64 = 86_400_000_000_000;

/// Simulate the account over `trips` = (close_ts_ns, return_on_notional_percent),
/// in chronological order. Compounds the balance; applies the daily-loss-limit
/// per UTC day.
#[must_use]
pub fn paper_run(trips: &[(u64, f64)], cfg: &PaperConfig) -> PaperResult {
    let mut bal = cfg.start_balance;
    let mut peak = bal;
    let mut max_dd = 0.0f64;
    let mut min_bal = bal;
    let mut taken = 0usize;
    let mut halted_days = 0usize;
    let mut ruined = false;

    let mut cur_day: Option<u64> = None;
    let mut day_start_bal = bal;
    let mut day_halted = false;

    for &(ts, pct) in trips {
        let day = ts / DAY_NS;
        if cur_day != Some(day) {
            cur_day = Some(day);
            day_start_bal = bal;
            day_halted = false;
        }
        if day_halted {
            continue;
        }
        if bal <= 0.0 {
            ruined = true;
            break;
        }

        let margin = bal * cfg.risk_pct;
        let notional = margin * cfg.leverage;
        bal += notional * (pct / 100.0);
        taken += 1;

        if bal < min_bal {
            min_bal = bal;
        }
        if bal <= 0.0 {
            bal = 0.0;
            ruined = true;
            break;
        }
        if bal > peak {
            peak = bal;
        }
        let dd = (peak - bal) / peak * 100.0;
        if dd > max_dd {
            max_dd = dd;
        }

        if cfg.daily_loss_limit_pct > 0.0 {
            let day_loss = (day_start_bal - bal) / day_start_bal;
            if day_loss >= cfg.daily_loss_limit_pct {
                day_halted = true;
                halted_days += 1;
            }
        }
    }

    PaperResult {
        start: cfg.start_balance,
        end: bal,
        return_pct: (bal / cfg.start_balance - 1.0) * 100.0,
        max_drawdown_pct: max_dd,
        min_balance: min_bal,
        trips_taken: taken,
        halted_days,
        ruined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PaperConfig {
        PaperConfig { start_balance: 500.0, leverage: 10.0, risk_pct: 0.1, daily_loss_limit_pct: 0.0 }
    }

    #[test]
    fn positive_edge_grows_balance() {
        // +0.1% per trip on 10x*10% sizing -> balance grows
        let trips: Vec<(u64, f64)> = (0..50).map(|i| (i * 1_000_000_000, 0.1)).collect();
        let r = paper_run(&trips, &cfg());
        assert!(r.end > r.start);
        assert!(r.return_pct > 0.0);
        assert!(!r.ruined);
    }

    #[test]
    fn negative_edge_shrinks_balance() {
        let trips: Vec<(u64, f64)> = (0..50).map(|i| (i * 1_000_000_000, -0.1)).collect();
        let r = paper_run(&trips, &cfg());
        assert!(r.end < r.start);
        assert!(r.max_drawdown_pct > 0.0);
    }

    #[test]
    fn daily_limit_halts_the_day() {
        // big losses same day; with a 5% daily limit, trading halts after the cap
        let c = PaperConfig { daily_loss_limit_pct: 0.05, ..cfg() };
        let trips: Vec<(u64, f64)> = (0..100).map(|_| (1_000_000_000u64, -1.0)).collect(); // all same day
        let r = paper_run(&trips, &c);
        assert_eq!(r.halted_days, 1);
        assert!(r.trips_taken < 100, "daily limit must stop further trades that day");
    }

    #[test]
    fn ruin_is_detected() {
        let c = PaperConfig { leverage: 50.0, risk_pct: 1.0, daily_loss_limit_pct: 0.0, ..cfg() };
        let trips: Vec<(u64, f64)> = (0..10).map(|i| (i * DAY_NS, -5.0)).collect(); // -250% notional/trip across days
        let r = paper_run(&trips, &c);
        assert!(r.ruined || r.end < 1.0);
    }
}