# percentile-kit

Latency percentile tracking and budget gates for Rust — the WyattAu estate's
P50/P99/P99.9 workflow: track latencies at runtime from a lock-free sliding
window, gate criterion benchmark regressions against budgets committed in
`percentile-budgets.toml`.

- **Lock-free hot path**: `record` is one relaxed `fetch_add` on a
  cache-line-padded write index plus one `Release` slot store — never
  blocks, locks, or allocates.
- **Sliding window, honestly documented**: fixed-capacity ring with
  oldest-sample overwrite; nearest-rank `P50`/`P99`/`P99.9`/max reads.
  NaN samples are rejected at append. For cumulative Prometheus histograms
  use [`metrics-kit`](https://github.com/WyattAu/metrics-kit).
- **Zero criterion dependency**: the report half parses criterion's
  `estimates.json` / `sample.json` with serde — P50 from the `median`
  point estimate, P99 nearest-rank over sampled data.
- **Committed budgets, CI enforcement**: `[[budget]]` entries in TOML with
  per-metric `max_regression_pct`; PASS/FAIL markdown tables for READMEs
  and a hard `ensure_pass()` gate for CI.
- **Loom-modelled concurrency**: two-producer/one-reader models prove
  ticket integrity (no torn index reads, no lost/duplicated appends) and
  snapshot exactness.
- **`#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`**, clippy
  `unwrap_used`/`expect_used`/`panic`/`indexing_slicing` denied.

## Install

```toml
[dependencies]
percentile-kit = "0.1"
```

## Example

```rust
use percentile_kit::PercentileTracker;

let tracker = PercentileTracker::<4096>::new();
for latency in [12.0, 8.0, 30.0, 25.0, 11.0] {
    tracker.record(latency);
}

assert_eq!(tracker.count(), 5);
assert_eq!(tracker.p50(), Some(12.0));
assert_eq!(tracker.p99(), Some(30.0));

// README-ready table row:
let row = tracker.render_markdown("ms");
assert!(row.contains("| P50 (ms) | P99 (ms) | P99.9 (ms) | Max (ms) | N |"));
```

## Budget gates for criterion benches

Commit the budgets next to your benches:

```toml
# percentile-budgets.toml
[[budget]]
metric = "fib_recursive"   # criterion bench directory under target/criterion
p50 = 100.0                # optional: budget for the median (ns, µs — your unit)
p99 = 250.0                # optional: budget for P99 (needs sample.json)
max_regression_pct = 10.0  # allowed excess, percent
```

Gate them in CI:

```rust,ignore
use std::path::Path;

let report = percentile_kit::check_budgets(
    Path::new("percentile-budgets.toml"),
    Path::new("target/criterion"),
)?;
println!("{}", report.to_markdown());
report.ensure_pass()?; // Err(BudgetExceeded{..}) fails the job
```

The canonical budget table (what `to_markdown()` emits and what estate
READMEs paste):

| Metric | P50 | P99 | Regression | Status |
|---|---|---|---|---|
| fib_recursive | 98.00 / ≤100.00 | 105.00 / ≤110.00 | -2.0% | PASS |
| sort_strings | 98.00 / ≤50.00 | n/a | +96.0% | FAIL |

Cells are `observed / ≤budget`; `n/a` means the value was not measurable
(no `sample.json`) or not budgeted; Regression is the worst excess across
gated cells.

## Performance

Measured with criterion on the committed bench suite (`cargo bench`); see
`benches/percentile_bench.rs`:

| Operation | Path | Cost model |
|---|---|---|
| `tracker.record(x)` | hot | 1 relaxed `fetch_add` + 1 `Release` store, padded index |
| `tracker.p99()` | read | O(N log N) snapshot under a mutex; appends never block |
| `parse_criterion_estimates` | CI | serde JSON, no criterion dependency |

Publish your measured numbers in your README per the estate standard.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `tracker` | yes | `PercentileTracker`, `CachePadded` |
| `report` | yes | criterion parsing, `Budget`, `check_budgets`, `ReportError` |
| `loom` | no | loom model doubles (`loom_tracker`) for exhaustive concurrency proofs |

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT)
at your option.
