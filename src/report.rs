//! Criterion report parsing and committed budget gates.
//!
//! Zero criterion dependency: this module only parses criterion's JSON
//! output (`target/criterion/<bench>/new/estimates.json` and
//! `sample.json`) with serde, compares the measured percentiles against
//! budgets committed in a `percentile-budgets.toml`, and renders a
//! PASS/FAIL markdown table for CI regression enforcement.
//!
//! # Budget file schema
//!
//! ```toml
//! [[budget]]
//! metric = "fib_recursive"   # criterion bench directory name
//! p50 = 100.0                # optional: budget for the median
//! p99 = 250.0                # optional: budget for the P99 (needs sample.json)
//! max_regression_pct = 10.0  # allowed excess over the budget, in percent
//! ```
//!
//! P50 is criterion's `median` point estimate. P99 is computed by
//! nearest-rank over the per-iteration times in `sample.json`; when the
//! sample file is absent, only budgets with a `p50` entry are gated.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::ReportError;
use crate::quantile::nearest_rank;

/// One bootstrap estimate group in criterion's `estimates.json` schema.
/// The confidence-interval fields are parsed (and required by the schema)
/// but not surfaced by the gate API.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct Estimate {
    confidence_interval: ConfidenceInterval,
    point_estimate: f64,
    standard_error: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct ConfidenceInterval {
    confidence_level: f64,
    lower_bound: f64,
    upper_bound: f64,
}

/// Parsed `target/criterion/<bench>/new/estimates.json`.
///
/// Deserialized tolerantly (unknown fields ignored) from criterion's
/// bootstrap-estimates schema; the `mean`, `median`, `slope`, and
/// `std_dev` groups are required.
#[derive(Debug, Clone, Deserialize)]
pub struct CriterionEstimates {
    mean: Estimate,
    median: Estimate,
    slope: Estimate,
    std_dev: Estimate,
}

impl CriterionEstimates {
    /// Mean per-iteration time point estimate.
    pub fn mean(&self) -> f64 {
        self.mean.point_estimate
    }

    /// Median per-iteration time point estimate.
    pub fn median(&self) -> f64 {
        self.median.point_estimate
    }

    /// P50 — the median point estimate (this crate's canonical P50).
    pub fn p50(&self) -> f64 {
        self.median()
    }

    /// Least-squares slope point estimate.
    pub fn slope(&self) -> f64 {
        self.slope.point_estimate
    }

    /// Standard deviation point estimate.
    pub fn std_dev(&self) -> f64 {
        self.std_dev.point_estimate
    }
}

/// Parses criterion's `estimates.json` for one bench.
///
/// `path` is typically `target/criterion/<bench>/new/estimates.json`. A
/// missing or unreadable file yields
/// [`ReportError::MissingEstimates`] (the bench name is inferred from the
/// path); a schema mismatch yields [`ReportError::InvalidJson`].
pub fn parse_criterion_estimates(path: &Path) -> Result<CriterionEstimates, ReportError> {
    let raw = fs::read_to_string(path).map_err(|_| ReportError::MissingEstimates {
        bench: bench_name_from_path(path),
    })?;
    serde_json::from_str(&raw).map_err(|source| ReportError::InvalidJson { source })
}

#[derive(Debug, Deserialize)]
struct CriterionSample {
    iters: Vec<f64>,
    times: Vec<f64>,
}

/// Parses criterion's `sample.json` into per-iteration times
/// (`times[i] / iters[i]`, iterations with non-positive iteration counts
/// skipped), suitable for [`nearest_rank`].
///
/// Errors mirror [`parse_criterion_estimates`].
pub fn parse_criterion_sample(path: &Path) -> Result<Vec<f64>, ReportError> {
    let raw = fs::read_to_string(path).map_err(|_| ReportError::MissingEstimates {
        bench: bench_name_from_path(path),
    })?;
    let sample: CriterionSample =
        serde_json::from_str(&raw).map_err(|source| ReportError::InvalidJson { source })?;
    Ok(sample
        .iters
        .iter()
        .zip(&sample.times)
        .filter(|(iters, _)| **iters > 0.0 && iters.is_finite())
        .map(|(iters, time)| time / iters)
        .collect())
}

/// Infers the bench name from a `.../criterion/<bench>/new/<file>` path.
fn bench_name_from_path(path: &Path) -> String {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// One committed budget for one criterion bench.
///
/// Deserialized from a `[[budget]]` entry in `percentile-budgets.toml`.
/// Unknown keys are rejected so typos (`p999` instead of `p99`) fail the
/// gate loudly instead of silently skipping a check.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Criterion bench directory name (e.g. `"fib_recursive"`).
    pub metric: String,
    /// Optional budget for P50 (criterion's median point estimate).
    pub p50: Option<f64>,
    /// Optional budget for P99 (requires `sample.json` next to
    /// `estimates.json`).
    pub p99: Option<f64>,
    /// Allowed excess over either budget, in percent
    /// (`0.0` = observed must not exceed the budget at all).
    pub max_regression_pct: f64,
}

#[derive(Debug, Deserialize)]
struct BudgetsFile {
    budget: Vec<Budget>,
}

fn load_budgets(path: &Path) -> Result<Vec<Budget>, ReportError> {
    let raw = fs::read_to_string(path).map_err(|err| ReportError::MalformedBudget {
        source: Box::new(err),
    })?;
    let file: BudgetsFile = toml::from_str(&raw).map_err(|err| ReportError::MalformedBudget {
        source: Box::new(err),
    })?;
    Ok(file.budget)
}

/// One bench's budget-gate result, rendered as one row of the README
/// budget table plus PASS/FAIL and the worst regression percent.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetRow {
    /// The budgeted metric (bench directory name).
    pub metric: String,
    /// Observed P50 (median point estimate), when available.
    pub p50_observed: Option<f64>,
    /// Committed P50 budget, when present.
    pub p50_budget: Option<f64>,
    /// Observed P99 (nearest-rank over sampled data), when `sample.json`
    /// was available.
    pub p99_observed: Option<f64>,
    /// Committed P99 budget, when present.
    pub p99_budget: Option<f64>,
    /// Worst (maximum) regression vs budget across *gated* cells, in
    /// percent — negative values are headroom. `0.0` when nothing was
    /// gated.
    pub regression_pct: f64,
    /// Whether every gated cell stayed within its budget plus
    /// `max_regression_pct`.
    pub pass: bool,
}

impl BudgetRow {
    /// Renders this row (no header) exactly as it appears in the README
    /// budget table: observed / ≤budget cells, the worst regression
    /// percent, and the PASS/FAIL status.
    pub fn to_markdown_row(&self) -> String {
        format!(
            "| {} | {} | {} | {:+.1}% | {} |",
            self.metric,
            cell(self.p50_observed, self.p50_budget),
            cell(self.p99_observed, self.p99_budget),
            self.regression_pct,
            if self.pass { "PASS" } else { "FAIL" }
        )
    }
}

/// Formats one budget-table cell: `observed / ≤budget` when gated,
/// `observed / —` when unbudgeted, `n/a` when unavailable.
fn cell(observed: Option<f64>, budget: Option<f64>) -> String {
    match (observed, budget) {
        (Some(o), Some(b)) => format!("{o:.2} / ≤{b:.2}"),
        (Some(o), None) => format!("{o:.2} / —"),
        (None, _) => "n/a".to_string(),
    }
}

/// Aggregated budget-gate report over all committed budgets.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetReport {
    /// Whether every row passed.
    pub pass: bool,
    /// One row per committed budget, in file order.
    pub rows: Vec<BudgetRow>,
}

impl BudgetReport {
    /// Renders the full README budget table (header, separator, one row
    /// per budget).
    pub fn to_markdown(&self) -> String {
        let mut out = String::from(
            "| Metric | P50 | P99 | Regression | Status |\n\
             |---|---|---|---|---|",
        );
        for row in &self.rows {
            out.push('\n');
            out.push_str(&row.to_markdown_row());
        }
        out
    }

    /// Hard-fail gate for CI: returns `Err(ReportError::BudgetExceeded)`
    /// for the first failing row (reporting its worst gated cell), or
    /// `Ok(())` when every row passed.
    pub fn ensure_pass(&self) -> Result<(), ReportError> {
        let row = match self.rows.iter().find(|row| !row.pass) {
            Some(row) => row,
            None => return Ok(()),
        };
        let gated = [
            (row.p50_observed, row.p50_budget),
            (row.p99_observed, row.p99_budget),
        ];
        let worst = gated
            .into_iter()
            .filter_map(|(observed, budget)| observed.zip(budget))
            .filter(|&(_, budget)| budget > 0.0)
            .map(|(observed, budget)| (observed, budget, excess_pct(observed, budget)))
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        match worst {
            Some((observed, budget, regression_pct)) => Err(ReportError::BudgetExceeded {
                metric: row.metric.clone(),
                observed,
                budget,
                regression_pct,
            }),
            // A row can only fail via a gated cell; unreachable in practice.
            None => Err(ReportError::BudgetExceeded {
                metric: row.metric.clone(),
                observed: f64::NAN,
                budget: f64::NAN,
                regression_pct: row.regression_pct,
            }),
        }
    }
}

fn excess_pct(observed: f64, budget: f64) -> f64 {
    (observed - budget) / budget * 100.0
}

/// Checks committed budgets against criterion output.
///
/// `budgets_path` points at the committed `percentile-budgets.toml`;
/// `criterion_dir` at criterion's output root (typically
/// `target/criterion`). Each budget's metric names a bench directory
/// beneath it. Parsed P50 is the median point estimate; P99 is computed
/// from `sample.json` when present, otherwise P99 budgets are not gated
/// for that bench. A budgeted bench with no `estimates.json` is a hard
/// [`ReportError::MissingEstimates`].
pub fn check_budgets(
    budgets_path: &Path,
    criterion_dir: &Path,
) -> Result<BudgetReport, ReportError> {
    let budgets = load_budgets(budgets_path)?;
    let mut rows = Vec::with_capacity(budgets.len());
    for budget in &budgets {
        let new_dir = criterion_dir.join(&budget.metric).join("new");
        let estimates = parse_criterion_estimates(&new_dir.join("estimates.json"))?;
        let p50_observed = Some(estimates.p50());
        let p99_observed = parse_criterion_sample(&new_dir.join("sample.json"))
            .ok()
            .and_then(|mut per_iter| {
                per_iter.sort_by(f64::total_cmp);
                nearest_rank(&per_iter, 0.99)
            });
        rows.push(evaluate(budget, p50_observed, p99_observed));
    }
    let pass = rows.iter().all(|row| row.pass);
    Ok(BudgetReport { pass, rows })
}

fn evaluate(budget: &Budget, p50_observed: Option<f64>, p99_observed: Option<f64>) -> BudgetRow {
    let allowed = budget.max_regression_pct;
    let gated = [(p50_observed, budget.p50), (p99_observed, budget.p99)];
    let mut worst = f64::NEG_INFINITY;
    let mut pass = true;
    for (observed, limit) in gated.into_iter().filter_map(|(o, l)| o.zip(l)) {
        let excess = excess_pct(observed, limit);
        worst = worst.max(excess);
        // NaN budgets/observations fail conservatively.
        if excess.is_nan() || excess > allowed {
            pass = false;
        }
    }
    let regression_pct = if worst.is_finite() { worst } else { 0.0 };
    BudgetRow {
        metric: budget.metric.clone(),
        p50_observed,
        p50_budget: budget.p50,
        p99_observed,
        p99_budget: budget.p99,
        regression_pct,
        pass,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn golden_estimates_parse() {
        let e = parse_criterion_estimates(&fixture("estimates.json")).expect("parse");
        assert_eq!(e.mean(), 99.0);
        assert_eq!(e.median(), 98.0);
        assert_eq!(e.p50(), 98.0);
        assert_eq!(e.slope(), 99.5);
        assert_eq!(e.std_dev(), 5.5);
    }

    #[test]
    fn golden_sample_p99_nearest_rank() {
        let per_iter = parse_criterion_sample(&fixture("sample.json")).expect("parse");
        let mut sorted = per_iter;
        sorted.sort_by(f64::total_cmp);
        assert_eq!(nearest_rank(&sorted, 0.99), Some(105.0));
        assert_eq!(nearest_rank(&sorted, 0.5), Some(100.0));
    }

    #[test]
    fn missing_estimates_carries_bench_name() {
        // A criterion-shaped path that does not exist on disk: the bench
        // name is inferred as `.../criterion/<bench>/new`.
        let path = Path::new("/tmp/target/criterion/ghost_bench/new/estimates.json");
        let err = parse_criterion_estimates(path).unwrap_err();
        assert!(
            matches!(err, ReportError::MissingEstimates { ref bench } if bench == "ghost_bench"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn invalid_json_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("b").join("new").join("estimates.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "{ not json").expect("write");
        let err = parse_criterion_estimates(&path).unwrap_err();
        assert!(matches!(err, ReportError::InvalidJson { .. }));
    }

    #[test]
    fn missing_bench_dir_reports_bench_name() {
        let err = parse_criterion_estimates(Path::new("/nonexistent/bench/new/estimates.json"))
            .unwrap_err();
        assert!(
            matches!(err, ReportError::MissingEstimates { ref bench } if bench == "bench"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn budget_evaluation_matrix() {
        let within = Budget {
            metric: "m".into(),
            p99: Some(110.0),
            p50: Some(100.0),
            max_regression_pct: 5.0,
        };
        let row = evaluate(&within, Some(98.0), Some(105.0));
        assert!(row.pass);
        assert!((row.regression_pct - (-2.0)).abs() < 1e-9); // max(-2%, -4.54%)

        let over = Budget {
            metric: "m".into(),
            p99: Some(100.0),
            p50: Some(100.0),
            max_regression_pct: 5.0,
        };
        let row = evaluate(&over, Some(98.0), Some(120.0));
        assert!(!row.pass); // P99 +20% > +5%
        assert!((row.regression_pct - 20.0).abs() < 1e-9);

        // p99 budgeted but no sample data → not gated, row passes.
        let no_sample = Budget {
            metric: "m".into(),
            p99: Some(100.0),
            p50: None,
            max_regression_pct: 5.0,
        };
        let row = evaluate(&no_sample, Some(98.0), None);
        assert!(row.pass);
        assert_eq!(row.regression_pct, 0.0);
    }

    #[test]
    fn markdown_row_snapshot() {
        let budget = Budget {
            metric: "fib_recursive".into(),
            p99: Some(110.0),
            p50: Some(100.0),
            max_regression_pct: 5.0,
        };
        let row = evaluate(&budget, Some(98.0), Some(105.0));
        assert_eq!(
            row.to_markdown_row(),
            "| fib_recursive | 98.00 / ≤100.00 | 105.00 / ≤110.00 | -2.0% | PASS |"
        );
        let report = BudgetReport {
            pass: true,
            rows: vec![row],
        };
        assert_eq!(
            report.to_markdown(),
            "| Metric | P50 | P99 | Regression | Status |\n\
             |---|---|---|---|---|\n\
             | fib_recursive | 98.00 / ≤100.00 | 105.00 / ≤110.00 | -2.0% | PASS |"
        );
    }

    #[test]
    fn ensure_pass_yields_budget_exceeded() {
        let report = BudgetReport {
            pass: false,
            rows: vec![BudgetRow {
                metric: "fib_recursive".into(),
                p50_observed: Some(98.0),
                p50_budget: Some(100.0),
                p99_observed: Some(120.0),
                p99_budget: Some(100.0),
                regression_pct: 20.0,
                pass: false,
            }],
        };
        let err = report.ensure_pass().unwrap_err();
        assert!(
            matches!(
                err,
                ReportError::BudgetExceeded {
                    ref metric,
                    observed,
                    budget,
                    regression_pct,
                } if metric == "fib_recursive"
                    && observed == 120.0
                    && budget == 100.0
                    && (regression_pct - 20.0).abs() < 1e-9
            ),
            "unexpected error: {err:?}"
        );
        assert!(BudgetReport {
            pass: true,
            rows: vec![]
        }
        .ensure_pass()
        .is_ok());
    }
}
