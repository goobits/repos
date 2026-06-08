//! Audit command internals for secret scanning, hygiene checks, and fixes.
//!
//! These modules back the `repos audit` CLI. Prefer the CLI for normal use; the
//! types remain public for integration tests and advanced automation.

pub mod fixes;
pub mod hygiene;
pub mod scanner;
