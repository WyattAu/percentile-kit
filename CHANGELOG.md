# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [0.1.0] - 2026-09-15

### Added

- `tracker` feature: `PercentileTracker<const N = 4096>` — fixed-capacity
  sliding window of `f64` samples with wait-free append (one relaxed
  `fetch_add` on a cache-line-padded write index + one `Release` slot
  store), NaN rejection, oldest-sample overwrite semantics, nearest-rank
  `quantile`/`p50`/`p99`/`p999`/`max` reads, and a README-ready
  `render_markdown(unit)` table.
- `report` feature: criterion `estimates.json`/`sample.json` parsing
  (zero criterion dependency), committed `percentile-budgets.toml`
  budgets with per-metric `max_regression_pct`, `check_budgets` gating,
  PASS/FAIL `BudgetReport` markdown tables, and a hard
  `BudgetReport::ensure_pass()` CI gate; `ReportError` (thiserror,
  `non_exhaustive`).
- Loom models (`loom` feature): two-producer/one-reader proofs of ticket
  integrity and snapshot exactness under every interleaving.
- Proptest suite (quantile vs sorted reference, 1000 cases; NaN
  rejection; count capping), golden-file report tests, budget
  pass/fail matrix, markdown snapshots, criterion benches
  (record throughput, full-ring quantile, report parse).
