//! The runtime's wall clock and blocking sleep.
//!
//! The assertions are deliberately loose: a test machine can be loaded, and a
//! wall clock can be adjusted. What must hold is that `now()` is a positive
//! millisecond value, that two calls do not go backwards in practice, and that
//! `sleep(30)` actually blocks for a measurable part of thirty milliseconds.

use std::time::Instant;

use yashy_runtime::{yashy_now_ms, yashy_sleep_ms};

#[test]
fn now_is_positive_and_does_not_run_backwards() {
    unsafe {
        let first = yashy_now_ms();
        let second = yashy_now_ms();
        assert!(first > 0, "the wall clock must be after the Unix epoch");
        assert!(second >= first, "two immediate reads must not go backwards");
    }
}

#[test]
fn sleeping_zero_and_negative_is_a_no_op() {
    let start = Instant::now();
    unsafe {
        yashy_sleep_ms(0);
        yashy_sleep_ms(-1);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 50,
        "a non-positive sleep returned after {elapsed:?}"
    );
}

#[test]
fn sleeping_thirty_milliseconds_blocks() {
    let start = Instant::now();
    unsafe {
        yashy_sleep_ms(30);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() >= 20,
        "sleep(30) returned after only {elapsed:?}"
    );
}
