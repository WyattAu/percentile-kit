//! Loom model double of the percentile tracker (compiled only with
//! `--features loom`).
//!
//! The double re-implements [`crate::tracker::PercentileTracker`]'s
//! protocol on loom's scheduler-tracked atomics, preserving the *identical*
//! ordering discipline of the real implementation:
//!
//! * producer: `Release` slot store **after** a `Relaxed` `fetch_add`
//!   ticket claim (index first — the slot lags the ticket, which is why
//!   the reader filters NaN-sentinel slots instead of trusting the index)
//! * reader: `Acquire` write-index load → `Acquire` slot loads → NaN-
//!   sentinel filtering
//!
//! What loom buys here is exhaustive exploration of every interleaving of
//! two producers and one reader for the properties under proof: tickets
//! are never lost or duplicated (no torn index reads), and a reader's
//! snapshot only ever contains exact, previously recorded samples — never
//! garbage, never torn values, never more than the window capacity.
//! Capacities and sample counts are deliberately tiny to keep the state
//! space tractable. The Mutex-protected snapshot scratch of the real
//! implementation is a single-reader serialization detail, not part of the
//! ordering protocol, and is omitted here (the models use one reader).

use loom::sync::atomic::{
    AtomicU64, AtomicUsize,
    Ordering::{Acquire, Relaxed, Release},
};
use loom::sync::Arc;

/// `f64::NAN.to_bits()` — the fill sentinel (mirrors the real tracker).
const NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

/// Loom double of [`crate::tracker::PercentileTracker`] (f64 payloads).
pub struct LoomTracker<const N: usize> {
    write_idx: AtomicUsize,
    slots: [AtomicU64; N],
}

impl<const N: usize> LoomTracker<N> {
    /// Fresh tracker with all slots NaN-sentinel filled.
    pub fn new() -> Self {
        Self {
            write_idx: AtomicUsize::new(0),
            slots: std::array::from_fn(|_| AtomicU64::new(NAN_BITS)),
        }
    }

    /// Identical protocol to [`crate::tracker::PercentileTracker::record`].
    pub fn record(&self, value: f64) {
        if value.is_nan() {
            return;
        }
        let ticket = self.write_idx.fetch_add(1, Relaxed);
        if let Some(slot) = self.slots.get(ticket % N) {
            slot.store(value.to_bits(), Release);
        }
    }

    /// Identical protocol to the real snapshot path: `Acquire` the write
    /// index, read the window's slots with `Acquire`, drop NaN sentinels.
    pub fn snapshot(&self) -> Vec<f64> {
        let end = self.write_idx.load(Acquire);
        let start = end.saturating_sub(N);
        (start..end)
            .filter_map(|ticket| {
                self.slots.get(ticket % N).and_then(|slot| {
                    let bits = slot.load(Acquire);
                    (bits != NAN_BITS).then(|| f64::from_bits(bits))
                })
            })
            .collect()
    }

    /// The raw write index (total tickets claimed).
    pub fn write_index(&self) -> usize {
        self.write_idx.load(Acquire)
    }

    /// The raw bits of one slot (for quiescent post-join assertions).
    pub fn slot_bits(&self, index: usize) -> u64 {
        self.slots.get(index).map_or(NAN_BITS, |s| s.load(Acquire))
    }
}

impl<const N: usize> Default for LoomTracker<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Propagates a panicked thread's payload instead of unwrapping the join
/// result (clippy `unwrap_used`/`expect_used` are denied crate-wide).
fn join_or_propagate<T>(handle: loom::thread::JoinHandle<T>) -> T {
    match handle.join() {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Runs a model exhaustively under a preemption bound of 2 (see the
/// module docs).
fn run_model<F>(f: F)
where
    F: Fn() + Sync + Send + 'static,
{
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(2);
    builder.check(f);
}

/// Model 1 — two producers, one reader, capacity 4: ticket integrity and
/// snapshot exactness.
///
/// Each producer records one distinct sample (A: 1.0, B: 2.0) while a
/// reader repeatedly snapshots. Under *every* interleaving loom can
/// generate:
///
/// * every value the reader observes is exactly one of the two recorded
///   samples — a torn, never-written, or garbage value would mean the
///   release/acquire slot protocol is broken;
/// * the NaN-sentinel filter never invents or drops completed samples
///   once writers quiesce; and
/// * after both producers finish, the write index is exactly 2 (the
///   relaxed `fetch_add` ticketing lost and duplicated no claims — no
///   torn index reads) and the quiescent window holds both samples
///   exactly once.
pub fn model_two_producers_reader_exactness() {
    const ATTEMPTS: usize = 2;
    run_model(move || {
        let tracker = Arc::new(LoomTracker::<4>::new());
        let mut handles = Vec::new();

        {
            let tracker = Arc::clone(&tracker);
            handles.push(loom::thread::spawn(move || {
                for _ in 0..ATTEMPTS {
                    for v in tracker.snapshot() {
                        assert!(
                            v == 1.0 || v == 2.0,
                            "reader observed a torn/garbage sample: {v}"
                        );
                    }
                }
            }));
        }

        for sample in [1.0, 2.0] {
            let tracker = Arc::clone(&tracker);
            handles.push(loom::thread::spawn(move || {
                tracker.record(sample);
            }));
        }

        for handle in handles {
            join_or_propagate(handle);
        }
        assert_eq!(
            tracker.write_index(),
            2,
            "every relaxed fetch_add ticket must survive exactly once"
        );
        let mut window = tracker.snapshot();
        window.sort_by(f64::total_cmp);
        assert_eq!(
            window,
            vec![1.0, 2.0],
            "quiescent window must hold every sample exactly once"
        );
    });
}

/// Model 2 — two producers, one reader, capacity 2: overwrite freshness
/// under a concurrent reader.
///
/// Producer A records 1.0; producer B records 2.0 then 4.0 — three
/// samples into two slots, so one slot is always overwritten, and when A
/// and B own tickets mapping to the same slot they race on it (tickets
/// 0 and 2 share slot 0). Which racing write survives is a scheduling
/// decision; the protocol guarantees an *exact recorded value* survives.
/// Under *every* interleaving:
///
/// * every mid-flight snapshot is bounded (≤ capacity) and exclusively
///   contains recorded samples — no torn or garbage values;
/// * the write index is exactly 3 — no lost or duplicated ticket claims,
///   no torn index reads; and
/// * once writers quiesce, both slots hold settled, non-sentinel, exactly
///   recorded samples (the overwrite landed).
pub fn model_overwrite_window_freshness() {
    const ATTEMPTS: usize = 2;
    run_model(move || {
        let tracker = Arc::new(LoomTracker::<2>::new());
        let mut handles = Vec::new();

        {
            let tracker = Arc::clone(&tracker);
            handles.push(loom::thread::spawn(move || {
                for _ in 0..ATTEMPTS {
                    let snapshot = tracker.snapshot();
                    assert!(snapshot.len() <= 2, "window exceeded capacity");
                    for v in snapshot {
                        assert!(
                            v == 1.0 || v == 2.0 || v == 4.0,
                            "reader observed a torn/garbage sample: {v}"
                        );
                    }
                }
            }));
        }

        {
            let tracker = Arc::clone(&tracker);
            handles.push(loom::thread::spawn(move || {
                tracker.record(1.0);
            }));
        }

        {
            let tracker = Arc::clone(&tracker);
            handles.push(loom::thread::spawn(move || {
                tracker.record(2.0);
                tracker.record(4.0);
            }));
        }

        for handle in handles {
            join_or_propagate(handle);
        }
        assert_eq!(
            tracker.write_index(),
            3,
            "every relaxed fetch_add ticket must survive exactly once"
        );
        for slot in 0..2 {
            let bits = tracker.slot_bits(slot);
            assert_ne!(bits, NAN_BITS, "slot {slot} unsettled after quiescence");
            let v = f64::from_bits(bits);
            assert!(
                v == 1.0 || v == 2.0 || v == 4.0,
                "slot {slot} holds a torn/garbage sample: {v}"
            );
        }
    });
}
