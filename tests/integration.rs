#![cfg(all(feature = "tracker", feature = "report"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests: tracker lifecycle under concurrency, and the full
//! committed-budget → criterion-output → PASS/FAIL gate on synthetic
//! criterion trees.

use percentile_kit::{check_budgets, PercentileTracker, ReportError};
use std::path::Path;
use std::sync::Arc;

const BUDGETS_TOML: &str = "\
[[budget]]
metric = \"fib_recursive\"
p50 = 100.0
p99 = 110.0
max_regression_pct = 5.0

[[budget]]
metric = \"sort_strings\"
p50 = 50.0
max_regression_pct = 0.0
";

/// Writes a synthetic criterion tree for one bench from the golden
/// fixtures.
fn write_bench(root: &Path, bench: &str, with_sample: bool) {
    let new_dir = root.join(bench).join("new");
    std::fs::create_dir_all(&new_dir).expect("mkdir");
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::fs::copy(
        format!("{manifest}/tests/fixtures/estimates.json"),
        new_dir.join("estimates.json"),
    )
    .expect("copy estimates");
    if with_sample {
        std::fs::copy(
            format!("{manifest}/tests/fixtures/sample.json"),
            new_dir.join("sample.json"),
        )
        .expect("copy sample");
    }
}

#[test]
fn tracker_service_lifecycle_under_concurrency() {
    let tracker = Arc::new(PercentileTracker::<1024>::new());
    let handles: Vec<_> = (0..4)
        .map(|worker| {
            let tracker = Arc::clone(&tracker);
            std::thread::spawn(move || {
                for i in 0..500u64 {
                    // 0.25-style binary-exact values keep order-independent
                    // sums deterministic; values stay in the worker's band.
                    tracker.record(1.0 + (worker * 500 + i) as f64 * 0.25);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("join");
    }

    assert_eq!(tracker.count(), 1024);
    let p50 = tracker.p50().expect("non-empty");
    let p99 = tracker.p99().expect("non-empty");
    let max = tracker.max().expect("non-empty");
    assert!(p50 <= p99 && p99 <= max);
    assert!(p99 <= tracker.p999().expect("non-empty"));

    let markdown = tracker.render_markdown("us");
    assert!(markdown.contains("| P50 (us) | P99 (us) | P99.9 (us) | Max (us) | N |"));
    assert!(markdown.contains("| 1024 |"));
}

#[test]
fn budget_gate_full_pass() {
    let dir = tempfile::tempdir().expect("tempdir");
    let criterion = dir.path().join("criterion");
    write_bench(&criterion, "fib_recursive", true);
    write_bench(&criterion, "sort_strings", false); // no sample.json → P99 n/a

    let budgets = dir.path().join("percentile-budgets.toml");
    std::fs::write(&budgets, BUDGETS_TOML).expect("write budgets");

    let report = check_budgets(&budgets, &criterion).expect("report");
    assert!(
        !report.pass,
        "sort_strings must fail the report: {report:?}"
    );
    assert_eq!(report.rows.len(), 2);
    let fib = report.rows.first().expect("two rows");
    assert_eq!(fib.metric, "fib_recursive");
    assert_eq!(fib.p50_observed, Some(98.0));
    assert_eq!(fib.p99_observed, Some(105.0));
    let sort = report.rows.get(1).expect("two rows");
    // sort_strings: P50 98.0 vs 50.0 budget at 0% allowed → FAIL.
    assert!(!sort.pass);
    assert!(!report.pass);
    assert!(report.ensure_pass().is_err());

    // Markdown snapshot of the full table.
    assert_eq!(
        report.to_markdown(),
        "| Metric | P50 | P99 | Regression | Status |\n\
         |---|---|---|---|---|\n\
         | fib_recursive | 98.00 / ≤100.00 | 105.00 / ≤110.00 | -2.0% | PASS |\n\
         | sort_strings | 98.00 / ≤50.00 | n/a | +96.0% | FAIL |"
    );
}

#[test]
fn budget_gate_all_within_budget_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let criterion = dir.path().join("criterion");
    write_bench(&criterion, "fib_recursive", true);

    let budgets = dir.path().join("percentile-budgets.toml");
    std::fs::write(
        &budgets,
        "[[budget]]\nmetric = \"fib_recursive\"\np50 = 100.0\np99 = 110.0\nmax_regression_pct = 5.0\n",
    )
    .expect("write budgets");

    let report = check_budgets(&budgets, &criterion).expect("report");
    assert!(report.pass);
    assert!(report.ensure_pass().is_ok());
}

#[test]
fn missing_bench_is_a_hard_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let criterion = dir.path().join("criterion");
    write_bench(&criterion, "fib_recursive", true);

    let budgets = dir.path().join("percentile-budgets.toml");
    std::fs::write(
        &budgets,
        "[[budget]]\nmetric = \"ghost_bench\"\np50 = 1.0\nmax_regression_pct = 0.0\n",
    )
    .expect("write budgets");

    let err = check_budgets(&budgets, &criterion).unwrap_err();
    assert!(
        matches!(err, ReportError::MissingEstimates { ref bench } if bench == "ghost_bench"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn malformed_budget_file_is_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let budgets = dir.path().join("percentile-budgets.toml");
    std::fs::write(&budgets, "[[budget]]\nmetricx = \"oops\"\n").expect("write budgets");

    let err = check_budgets(&budgets, dir.path().join("criterion").as_path()).unwrap_err();
    assert!(matches!(err, ReportError::MalformedBudget { .. }));
}

#[test]
fn corrupted_estimates_json_is_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let criterion = dir.path().join("criterion");
    let new_dir = criterion.join("fib_recursive").join("new");
    std::fs::create_dir_all(&new_dir).expect("mkdir");
    std::fs::write(new_dir.join("estimates.json"), "[]").expect("write");

    let budgets = dir.path().join("percentile-budgets.toml");
    std::fs::write(
        &budgets,
        "[[budget]]\nmetric = \"fib_recursive\"\np50 = 1.0\nmax_regression_pct = 0.0\n",
    )
    .expect("write budgets");

    let err = check_budgets(&budgets, &criterion).unwrap_err();
    assert!(matches!(err, ReportError::InvalidJson { .. }));
}
