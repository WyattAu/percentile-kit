//! Latency percentile tracking and budget gates.
//!
//! `percentile-kit` covers the two halves of the WyattAu latency-gate
//! workflow:
//!
//! 1. **Runtime tracking** (`tracker`): a fixed-capacity, lock-free-append
//!    sliding window of `f64` samples (any unit — the caller's choice) with
//!    nearest-rank `P50`/`P99`/`P99.9`/max reads and a README-ready
//!    markdown row renderer.
//! 2. **Criterion report gating** (`report`): parse criterion's
//!    `estimates.json`/`sample.json` output (zero criterion dependency),
//!    compare against budgets committed in a `percentile-budgets.toml`,
//!    and render a PASS/FAIL table for CI regression enforcement.
//!
//! # Design
//!
//! - **Hot path is lock-free.** [`PercentileTracker::record`] is one
//!   relaxed `fetch_add` on a cache-line-padded write index plus one
//!   `Release` slot store. It never blocks, locks, or allocates.
//! - **Windows, not histograms.** The tracker is a sliding window (oldest
//!   samples are overwritten once the ring is full); for cumulative
//!   Prometheus histograms use `metrics-kit`.
//! - **Zero criterion dependency.** The report half parses criterion's
//!   JSON output with `serde`; budgets live in a committed TOML file so
//!   regressions gate pull requests, not vibes.
//!
//! # Example
//!
//! ```
//! use percentile_kit::PercentileTracker;
//!
//! let tracker = PercentileTracker::<64>::new();
//! for latency in [12.0, 8.0, 30.0, 25.0, 11.0] {
//!     tracker.record(latency);
//! }
//!
//! assert_eq!(tracker.count(), 5);
//! assert_eq!(tracker.p50(), Some(12.0));
//! assert_eq!(tracker.max(), Some(30.0));
//! assert_eq!(tracker.quantile(f64::NAN), None); // invalid q → None
//!
//! let row = tracker.render_markdown("ms");
//! assert!(row.contains("| P50 (ms) | P99 (ms) | P99.9 (ms) | Max (ms) | N |"));
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod quantile;

pub use quantile::nearest_rank;

#[cfg(feature = "tracker")]
mod padding;
#[cfg(feature = "tracker")]
mod tracker;

#[cfg(feature = "tracker")]
pub use padding::CachePadded;
#[cfg(feature = "tracker")]
pub use tracker::PercentileTracker;

#[cfg(feature = "report")]
mod error;
#[cfg(feature = "report")]
mod report;

#[cfg(feature = "report")]
pub use error::ReportError;
#[cfg(feature = "report")]
pub use report::{
    check_budgets, parse_criterion_estimates, parse_criterion_sample, Budget, BudgetReport,
    BudgetRow, CriterionEstimates,
};

#[cfg(feature = "loom")]
pub mod loom_tracker;
