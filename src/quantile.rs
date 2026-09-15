//! Nearest-rank percentile over sorted samples.
//!
//! Shared by the runtime tracker (which sorts a snapshot of the ring) and
//! the report gate (which computes P99 from criterion's sampled data), so
//! both halves of the crate use one rank definition.

/// Nearest-rank percentile of a **sorted-ascending** sample slice.
///
/// The rank is `ceil(q * n)` clamped to `[1, n]`; the result is the
/// rank-th smallest sample. `q = 0.0` yields the minimum, `q = 1.0` the
/// maximum. Returns `None` for an empty slice or for `q` outside
/// `0.0..=1.0` (including NaN).
///
/// Passing an unsorted slice is a logic error: the function does not sort
/// (and in debug builds asserts sortedness) because both in-crate callers
/// sort exactly once per snapshot.
pub fn nearest_rank(sorted: &[f64], q: f64) -> Option<f64> {
    if !(0.0..=1.0).contains(&q) {
        return None;
    }
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let rank = (q * n as f64).ceil().clamp(1.0, n as f64) as usize;
    debug_assert!(
        sorted.windows(2).all(|w| matches!(w, [a, b] if a <= b)),
        "nearest_rank requires a sorted-ascending slice"
    );
    sorted.get(rank - 1).copied()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn empty_slice_is_none() {
        assert_eq!(nearest_rank(&[], 0.5), None);
    }

    #[test]
    fn invalid_q_is_none() {
        let s = [1.0, 2.0, 3.0];
        assert_eq!(nearest_rank(&s, -0.1), None);
        assert_eq!(nearest_rank(&s, 1.1), None);
        assert_eq!(nearest_rank(&s, f64::NAN), None);
    }

    #[test]
    fn ranks_hit_min_median_max() {
        let s = [10.0, 20.0, 30.0, 40.0];
        assert_eq!(nearest_rank(&s, 0.0), Some(10.0));
        assert_eq!(nearest_rank(&s, 0.5), Some(20.0)); // ceil(2.0) = 2
        assert_eq!(nearest_rank(&s, 0.51), Some(30.0)); // ceil(2.04) = 3
        assert_eq!(nearest_rank(&s, 1.0), Some(40.0));
    }

    #[test]
    fn single_sample_is_every_quantile() {
        assert_eq!(nearest_rank(&[42.0], 0.0), Some(42.0));
        assert_eq!(nearest_rank(&[42.0], 0.999), Some(42.0));
        assert_eq!(nearest_rank(&[42.0], 1.0), Some(42.0));
    }
}
