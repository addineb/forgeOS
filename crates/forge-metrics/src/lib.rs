//! `forge-metrics` - pure validation statistics for separating real edge from
//! curve-fit. No engine deps; everything operates on `f64` return/P&L series.
//!
//! - [`stats`]: mean / std / skew / kurtosis / Sharpe and an edge summary.
//! - [`normal`]: standard-normal CDF and inverse-CDF (for the Sharpe tests).
//! - [`dsr`]: the Deflated Sharpe Ratio (penalises how many configs were tried).
//! - [`pbo`]: Probability of Backtest Overfitting via CSCV.

#![forbid(unsafe_code)]

pub mod dsr;
pub mod normal;
pub mod paper;
pub mod pbo;
pub mod stats;

pub use dsr::{deflated_sharpe, expected_max_sharpe, probabilistic_sharpe, variance_of_sharpes};
pub use normal::{inv_normal_cdf, normal_cdf};
pub use paper::{paper_run, PaperConfig, PaperResult};
pub use pbo::{pbo_cscv, PboResult};
pub use stats::{kurtosis, mean, sharpe, skewness, std_dev, EdgeStats};