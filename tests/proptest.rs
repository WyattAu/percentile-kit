#![cfg(feature = "tracker")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Property tests: tracker quantiles vs an independently implemented
//! sorted-reference nearest rank, across ring sizes and windows.

use percentile_kit::PercentileTracker;
use proptest::prelude::*;

/// Finite f64 samples (NaN is rejected by the tracker by contract;
/// infinities are legal samples but excluded here to keep the reference
/// arithmetic unambiguous).
fn finite() -> impl Strategy<Value = f64> {
    any::<f64>().prop_filter("finite", |v| v.is_finite())
}

/// Reference nearest-rank on a freshly sorted copy.
fn reference(mut window: Vec<f64>, q: f64) -> Option<f64> {
    if !(0.0..=1.0).contains(&q) || window.is_empty() {
        return None;
    }
    window.sort_by(f64::total_cmp);
    let n = window.len();
    let rank = (q * n as f64).ceil().clamp(1.0, n as f64) as usize;
    window.get(rank - 1).copied()
}

fn check_against_reference<const N: usize>(vals: &[f64], q: f64) {
    let tracker = PercentileTracker::<N>::new();
    for &v in vals {
        tracker.record(v);
    }

    // Window = the N newest samples.
    let mut window: Vec<f64> = vals.to_vec();
    if window.len() > N {
        let drop = window.len() - N;
        window.drain(..drop);
    }

    assert_eq!(tracker.quantile(q), reference(window, q));
    assert_eq!(tracker.count(), vals.len().min(N));
    // Out-of-range q is always None.
    assert_eq!(tracker.quantile(-0.1), None);
    assert_eq!(tracker.quantile(1.1), None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn quantile_matches_sorted_reference(
        vals in prop::collection::vec(finite(), 0..300),
        q in 0.0f64..=1.0,
        size in 1usize..=4,
    ) {
        match size {
            1 => check_against_reference::<4>(&vals, q),
            2 => check_against_reference::<16>(&vals, q),
            3 => check_against_reference::<64>(&vals, q),
            _ => check_against_reference::<128>(&vals, q),
        }
    }

    #[test]
    fn nan_never_enters_the_window(
        nans in prop::collection::vec(Just(f64::NAN), 0..50),
        good in finite(),
    ) {
        let tracker = PercentileTracker::<16>::new();
        for v in &nans {
            tracker.record(*v);
        }
        tracker.record(good);
        assert_eq!(tracker.count(), 1);
        assert_eq!(tracker.p50(), Some(good));
        assert_eq!(tracker.max(), Some(good));
    }

    #[test]
    fn count_is_monotone_and_capped(
        vals in prop::collection::vec(finite(), 0..200),
    ) {
        let tracker = PercentileTracker::<32>::new();
        let mut expected = 0usize;
        for v in &vals {
            tracker.record(*v);
            expected += 1;
            assert_eq!(tracker.count(), expected.min(32));
        }
    }
}
