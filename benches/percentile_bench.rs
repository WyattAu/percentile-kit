//! Criterion benches: hot-path recording, full-ring quantile reads, and
//! report parsing.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use criterion::{criterion_group, criterion_main, Criterion};
use percentile_kit::{CriterionEstimates, PercentileTracker};
use std::hint::black_box;

const ESTIMATES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/estimates.json"
));

fn bench_record_overwrite(c: &mut Criterion) {
    let tracker = PercentileTracker::<4096>::new();
    for v in 0..4096 {
        tracker.record(v as f64);
    }
    c.bench_function("record_overwrite_full_ring", |b| {
        b.iter(|| black_box(&tracker).record(black_box(1.5)))
    });
}

fn bench_quantile_full_ring(c: &mut Criterion) {
    let tracker = PercentileTracker::<4096>::new();
    for v in 0..4096 {
        tracker.record(v as f64);
    }
    c.bench_function("quantile_p99_full_ring", |b| {
        b.iter(|| black_box(&tracker).p99())
    });
}

fn bench_render_markdown(c: &mut Criterion) {
    let tracker = PercentileTracker::<4096>::new();
    for v in 0..4096 {
        tracker.record(v as f64);
    }
    c.bench_function("render_markdown_full_ring", |b| {
        b.iter(|| black_box(&tracker).render_markdown("ns"))
    });
}

fn bench_parse_estimates(c: &mut Criterion) {
    c.bench_function("parse_criterion_estimates", |b| {
        b.iter(|| serde_json::from_str::<CriterionEstimates>(black_box(ESTIMATES_JSON)))
    });
}

criterion_group!(
    benches,
    bench_record_overwrite,
    bench_quantile_full_ring,
    bench_render_markdown,
    bench_parse_estimates
);
criterion_main!(benches);
