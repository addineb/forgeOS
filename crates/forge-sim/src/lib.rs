//! `forge-sim` - the deterministic replay core (Phase 1 skeleton).
//!
//! Drives a [`Strategy`] over an event stream with a virtual clock
//! (`now == event.local_ts`) and the two-clock latency model: inbound feed
//! latency is already baked into `local_ts` at conversion, and outbound order
//! latency is applied here (an order submitted at T reaches the matching engine
//! at `T + order_latency`). No-lookahead is structural: a strategy is only ever
//! handed data whose `local_ts <= now`. The matching engine + fills land in
//! Phase 2; this skeleton wires the clock, loop, latency queue, a determinism
//! hash, and the no-lookahead guarantee.

#![forbid(unsafe_code)]

mod account;
mod coinflip;
mod engine;
mod fills;
mod strategy;

pub use account::Account;
pub use coinflip::Coinflip;
pub use engine::{SimConfig, SimEngine, SimReport};
pub use fills::{maker_fill, money_to_f64, price_market, price_to_limit, FeeSchedule, Fill, Money};
pub use strategy::{Ctx, NoopStrategy, OrderIntent, OrderKind, Strategy};