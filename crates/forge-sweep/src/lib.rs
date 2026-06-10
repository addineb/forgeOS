//! `forge-sweep` - run a grid of strategy configs across windows in parallel,
//! score each honestly (net, Sharpe, Deflated Sharpe, sweep PBO) and assign a
//! promote / park / retire verdict with TUNABLE two-bar thresholds: lenient to
//! KEEP a candidate alive, strict only to send it LIVE, with a hard floor
//! (net <= 0, too few trades, DSR < 0.5, or PBO >= 0.5 => retire).
//!
//! Parallelism: rayon fans configs over the shared (already-decoded) event
//! windows; each run is an independent deterministic sim, so results are
//! identical regardless of thread count (aggregated in config-id order).

#![forbid(unsafe_code)]

mod grid;
mod run;

pub use grid::{expand, expand_cvd, expand_imbalance, expand_wallflow, CvdGridSpec, GridSpec, ImbalanceGridSpec, WallFlowGridSpec};
pub use run::{run_sweep, CellResult, SweepReport, Thresholds, Verdict};