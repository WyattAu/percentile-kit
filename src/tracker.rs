//! Fixed-capacity sliding-window percentile tracker.
//!
//! [`PercentileTracker`] holds up to `N` `f64` samples (latencies in any
//! unit — the caller's choice) in a ring, with lock-free append and
//! nearest-rank quantile reads.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::padding::CachePadded;
use crate::quantile::nearest_rank;

/// `f64::NAN.to_bits()` — the fill sentinel for "slot empty or append in
/// flight". Safe as a sentinel because NaN samples are rejected at append,
/// so no recorded sample can ever carry this bit pattern.
const NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

/// Fixed-capacity sliding window of `f64` samples with nearest-rank
/// percentile reads.
///
/// The ring starts filled with a NaN sentinel; `record` claims a monotonically
/// increasing ticket and overwrites the ticket's slot, so once `N` samples
/// have been recorded the oldest samples are overwritten — these are
/// **window semantics**, not a histogram. Use `metrics-kit`'s `Histogram`
/// for cumulative, scrape-friendly latency aggregates.
///
/// When two concurrent appends race for the same slot, either write may
/// survive that round (a scheduling decision, not a guarantee) — the
/// window is a sampler, and every surviving sample is exact. Under
/// non-racing (or single-writer) appends the window is exactly the `N`
/// most recent samples.
///
/// # Concurrency contract
///
/// - `record` is **wait-free**: exactly one relaxed `fetch_add` on the
///   cache-line-padded write index (claiming a unique ticket) plus one
///   `Release` store of the sample bits into the ticket's slot. It never
///   blocks, locks, or allocates, and never observes the snapshot mutex.
/// - `quantile` (and the `p50`/`p99`/`p999`/`max` helpers) take a Mutex
///   that protects the scratch snapshot buffer only, so concurrent quantile
///   computations serialize with *each other*; concurrent `record` calls
///   are never blocked.
/// - A snapshot is a point-in-time view of the ring: slots are atomics, so
///   every sample it contains is an **exact** previously recorded value —
///   no torn reads are possible. An append whose ticket was claimed but
///   whose slot store has not yet landed is filtered out (the NaN
///   sentinel) or included, depending on interleaving. Once writers
///   quiesce, the next snapshot is exact.
///
/// # NaN and validity
///
/// NaN samples are rejected at append (a no-op). `quantile` returns `None`
/// for an empty tracker or `q` outside `0.0..=1.0` (NaN included).
///
/// # Example
///
/// ```
/// use percentile_kit::PercentileTracker;
///
/// let tracker = PercentileTracker::<64>::new();
/// for latency in [12.0, 8.0, 30.0, 25.0, 11.0] {
///     tracker.record(latency);
/// }
///
/// assert_eq!(tracker.count(), 5);
/// assert_eq!(tracker.p50(), Some(12.0));
/// assert_eq!(tracker.p99(), Some(30.0));
/// assert_eq!(tracker.max(), Some(30.0));
/// ```
#[derive(Debug)]
pub struct PercentileTracker<const N: usize = 4096> {
    write_idx: CachePadded<AtomicUsize>,
    slots: Box<[AtomicU64]>,
    snapshot: Mutex<SnapshotScratch>,
}

#[derive(Debug, Default)]
struct SnapshotScratch {
    samples: Vec<f64>,
}

impl<const N: usize> PercentileTracker<N> {
    /// Creates an empty tracker with capacity `N`.
    pub fn new() -> Self {
        let mut slots = Vec::with_capacity(N);
        slots.resize_with(N, || AtomicU64::new(NAN_BITS));
        Self {
            write_idx: CachePadded::new(AtomicUsize::new(0)),
            slots: slots.into_boxed_slice(),
            snapshot: Mutex::new(SnapshotScratch::default()),
        }
    }

    /// Records one sample.
    ///
    /// Wait-free: one relaxed `fetch_add` claims the ticket, one `Release`
    /// store lands the value. NaN samples are rejected (a no-op) — see the
    /// type-level contract. Once the ring is full, the oldest sample is
    /// overwritten (sliding window); racing appends targeting the same
    /// slot each land exactly, and either may win the round.
    pub fn record(&self, value: f64) {
        if value.is_nan() {
            return;
        }
        let ticket = self.write_idx.fetch_add(1, Ordering::Relaxed);
        if let Some(slot) = self.slots.get(ticket % N) {
            slot.store(value.to_bits(), Ordering::Release);
        }
    }

    /// Number of samples in the window: `min(total records, N)`.
    ///
    /// Exact once writers quiesce; under concurrent appends it is a lower
    /// bound on the records issued. The underlying write index wraps at
    /// `usize::MAX` (unreachable in practice — 584 years at one record per
    /// nanosecond); power-of-two `N` keeps even that theoretical wrap
    /// slot-consistent.
    pub fn count(&self) -> usize {
        self.write_idx.load(Ordering::Relaxed).min(N)
    }

    /// Nearest-rank quantile of the current window, `q` in `0.0..=1.0`.
    ///
    /// Returns `None` for an empty tracker or invalid `q`. See the
    /// type-level concurrency contract for snapshot semantics.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        if !(0.0..=1.0).contains(&q) {
            return None;
        }
        let mut scratch = self.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
        scratch.samples.clear();
        let end = self.write_idx.load(Ordering::Acquire);
        let start = end.saturating_sub(N);
        for ticket in start..end {
            if let Some(slot) = self.slots.get(ticket % N) {
                let bits = slot.load(Ordering::Acquire);
                if bits != NAN_BITS {
                    scratch.samples.push(f64::from_bits(bits));
                }
            }
        }
        scratch.samples.sort_by(f64::total_cmp);
        nearest_rank(&scratch.samples, q)
    }

    /// 50th percentile (median) of the window.
    pub fn p50(&self) -> Option<f64> {
        self.quantile(0.5)
    }

    /// 99th percentile of the window.
    pub fn p99(&self) -> Option<f64> {
        self.quantile(0.99)
    }

    /// 99.9th percentile of the window.
    pub fn p999(&self) -> Option<f64> {
        self.quantile(0.999)
    }

    /// Maximum sample of the window.
    pub fn max(&self) -> Option<f64> {
        self.quantile(1.0)
    }

    /// Renders the window as a standards-README-ready markdown table:
    /// a `| P50 | P99 | P99.9 | Max | N |` header (annotated with
    /// `unit`), a separator row, and one data row. An empty tracker
    /// renders `-` in the value cells and `0` for `N`.
    pub fn render_markdown(&self, unit: &str) -> String {
        let fmt = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |x| format!("{x:.2}"));
        format!(
            "| P50 ({unit}) | P99 ({unit}) | P99.9 ({unit}) | Max ({unit}) | N |\n\
             |---|---|---|---|---|\n\
             | {} | {} | {} | {} | {} |",
            fmt(self.p50()),
            fmt(self.p99()),
            fmt(self.p999()),
            fmt(self.max()),
            self.count()
        )
    }
}

impl<const N: usize> Default for PercentileTracker<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::Arc;

    #[test]
    fn empty_tracker_yields_none_everywhere() {
        let t = PercentileTracker::<8>::new();
        assert_eq!(t.quantile(0.5), None);
        assert_eq!(t.p50(), None);
        assert_eq!(t.p99(), None);
        assert_eq!(t.p999(), None);
        assert_eq!(t.max(), None);
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn quantiles_match_sorted_reference() {
        let t = PercentileTracker::<64>::new();
        for v in [12.0, 8.0, 30.0, 25.0, 11.0] {
            t.record(v);
        }
        assert_eq!(t.count(), 5);
        assert_eq!(t.p50(), Some(12.0)); // ceil(0.5 * 5) = 3rd of [8,11,12,25,30]
        assert_eq!(t.p99(), Some(30.0)); // ceil(0.99 * 5) = 5th
        assert_eq!(t.p999(), Some(30.0));
        assert_eq!(t.max(), Some(30.0));
        assert_eq!(t.quantile(0.0), Some(8.0));
    }

    #[test]
    fn invalid_q_is_none() {
        let t = PercentileTracker::<8>::new();
        t.record(1.0);
        assert_eq!(t.quantile(-0.1), None);
        assert_eq!(t.quantile(1.1), None);
        assert_eq!(t.quantile(f64::NAN), None);
    }

    #[test]
    fn nan_samples_are_rejected() {
        let t = PercentileTracker::<8>::new();
        t.record(f64::NAN);
        t.record(-f64::NAN);
        t.record(7.5);
        t.record(f64::NAN);
        assert_eq!(t.count(), 1);
        assert_eq!(t.p50(), Some(7.5));
    }

    #[test]
    fn full_ring_overwrites_oldest_sample() {
        let t = PercentileTracker::<4>::new();
        for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
            t.record(v);
        }
        assert_eq!(t.count(), 4);
        // Window holds the four newest: [3, 4, 5, 6].
        assert_eq!(t.quantile(0.0), Some(3.0));
        assert_eq!(t.p50(), Some(4.0)); // ceil(0.5 * 4) = 2nd of [3,4,5,6]
        assert_eq!(t.quantile(0.99), Some(6.0)); // ceil(0.99 * 4) = 4th
        assert_eq!(t.max(), Some(6.0));
    }

    #[test]
    fn default_is_4096_capacity() {
        let t = PercentileTracker::<4096>::default();
        for v in 0..5_000 {
            t.record(v as f64);
        }
        assert_eq!(t.count(), 4096);
        assert_eq!(t.quantile(0.0), Some(904.0)); // oldest survivor is #904
    }

    #[test]
    fn markdown_render_snapshot() {
        let t = PercentileTracker::<64>::new();
        for v in [12.0, 8.0, 30.0, 25.0, 11.0] {
            t.record(v);
        }
        let expected = "| P50 (ms) | P99 (ms) | P99.9 (ms) | Max (ms) | N |\n\
                        |---|---|---|---|---|\n\
                        | 12.00 | 30.00 | 30.00 | 30.00 | 5 |";
        assert_eq!(t.render_markdown("ms"), expected);
    }

    #[test]
    fn empty_markdown_render_snapshot() {
        let t = PercentileTracker::<8>::new();
        let expected = "| P50 (ns) | P99 (ns) | P99.9 (ns) | Max (ns) | N |\n\
                        |---|---|---|---|---|\n\
                        | - | - | - | - | 0 |";
        assert_eq!(t.render_markdown("ns"), expected);
    }

    #[test]
    fn concurrent_records_are_never_lost() {
        let t = Arc::new(PercentileTracker::<4096>::new());
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let t = Arc::clone(&t);
                std::thread::spawn(move || {
                    for i in 0..1_000u64 {
                        t.record(1.0 + (worker * 1_000 + i) as f64);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("join");
        }
        assert_eq!(t.count(), 4096);
        // Window holds 4096 of the 8000 recorded values — all exact,
        // unique, from the recorded set.
        let mut seen = std::collections::HashSet::new();
        for ticket in 0..4096 {
            let bits = t
                .slots
                .get(ticket)
                .map_or(0, |slot| slot.load(Ordering::Acquire));
            assert_ne!(bits, NAN_BITS, "quiescent window must be fully written");
            let v = f64::from_bits(bits);
            assert!((1.0..=8000.0).contains(&v), "garbage sample {v}");
            assert!(seen.insert(bits), "duplicate sample {v}");
        }
    }
}
