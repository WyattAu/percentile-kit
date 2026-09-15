//! Loom model tests (run with `cargo test --features loom`).
//!
//! The models live in [`percentile_kit::loom_tracker`]: an in-memory double
//! of the tracker that keeps the identical relaxed-ticket / release-store /
//! acquire-read ordering discipline while letting loom exhaustively explore
//! interleavings of two producers and one reader.

#![cfg(feature = "loom")]

#[test]
fn loom_two_producers_reader_exactness() {
    percentile_kit::loom_tracker::model_two_producers_reader_exactness();
}

#[test]
fn loom_overwrite_window_freshness() {
    percentile_kit::loom_tracker::model_overwrite_window_freshness();
}
