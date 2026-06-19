//! Registry of active causal templates.
//!
//! Currently only `absorption_reversal` is implemented. Add more templates
//! by writing a struct in this directory that implements `CausalTemplate`,
//! and re-exporting it here.

pub mod absorption_reversal;

pub use absorption_reversal::{AbsorptionReversalTemplate, describe_absorption_reversal};
