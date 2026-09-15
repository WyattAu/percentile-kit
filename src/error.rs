//! Errors produced by criterion report parsing and budget gating.

use thiserror::Error;

/// Report parsing and budget gate failures.
///
/// Documented failure modes are part of the API contract; every variant
/// lists the condition that produces it.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReportError {
    /// A criterion bench has no readable `estimates.json` (the expected
    /// layout is `target/criterion/<bench>/new/estimates.json`). Also
    /// raised for unreadable `sample.json` when parsing sample data
    /// directly.
    #[error("criterion estimates missing or unreadable for bench {bench:?}")]
    MissingEstimates {
        /// The bench name (directory under `target/criterion`) that has no
        /// estimates.
        bench: String,
    },
    /// A criterion JSON file exists but does not match the expected
    /// schema.
    #[error("invalid criterion report JSON")]
    InvalidJson {
        /// The underlying serde failure.
        #[source]
        source: serde_json::Error,
    },
    /// A budgeted metric exceeded its allowed regression. Raised by
    /// [`BudgetReport::ensure_pass`](crate::BudgetReport::ensure_pass) for
    /// CI hard-fail semantics; the row-level view is
    /// [`BudgetReport`](crate::BudgetReport).
    #[error(
        "budget exceeded for {metric}: observed {observed} > budget {budget} \
         (+{regression_pct:.1}% regression)"
    )]
    BudgetExceeded {
        /// The budgeted metric that failed.
        metric: String,
        /// The observed (measured) value.
        observed: f64,
        /// The committed budget.
        budget: f64,
        /// The regression vs budget, in percent.
        regression_pct: f64,
    },
    /// The budgets file is unreadable or does not parse as the documented
    /// `percentile-budgets.toml` schema.
    #[error("malformed percentile-budgets file")]
    MalformedBudget {
        /// The underlying IO or TOML parse failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::error::Error as _;

    #[test]
    fn display_is_informative() {
        let e = ReportError::MissingEstimates {
            bench: "fib_recursive".into(),
        };
        assert_eq!(
            e.to_string(),
            "criterion estimates missing or unreadable for bench \"fib_recursive\""
        );

        let e = ReportError::BudgetExceeded {
            metric: "fib_recursive".into(),
            observed: 110.0,
            budget: 100.0,
            regression_pct: 10.0,
        };
        assert_eq!(
            e.to_string(),
            "budget exceeded for fib_recursive: observed 110 > budget 100 (+10.0% regression)"
        );

        let e = ReportError::MalformedBudget {
            source: "toml exploded".into(),
        };
        assert_eq!(e.to_string(), "malformed percentile-budgets file");
        assert_eq!(
            e.source().map(|s| s.to_string()),
            Some("toml exploded".into())
        );
    }
}
