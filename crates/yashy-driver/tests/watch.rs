//! `watch` reports diagnostics on change and a success event when fixed.
//!
//! The test drives `Watcher` directly: it is the loop's only interesting part,
//! and polling it by hand keeps the test deterministic instead of racing a
//! sleeping thread.

mod common;

use common::{work_dir, write};
use std::path::PathBuf;
use std::time::Instant;
use yashy_driver::watch::{WatchEvent, Watcher};

fn diagnostics(event: &WatchEvent) -> Option<&Vec<yashy_common::json_rpc::Value>> {
    match event {
        WatchEvent::Diagnostics { diagnostics, .. } => Some(diagnostics),
        _ => None,
    }
}

#[test]
fn a_change_reports_diagnostics_and_a_fix_reports_success() {
    let dir = work_dir("watch-diagnostics");
    write(&dir, "main.ya", "fn main() {}\n");

    let mut watcher = Watcher::new(&dir, None).expect("the watcher must start");
    assert!(watcher.poll().is_empty(), "an untouched tree is quiet");

    // Introduce a type error; the very next poll must report it.
    write(
        &dir,
        "main.ya",
        "fn main() {\n    const int count = \"three\";\n}\n",
    );
    let started = Instant::now();
    let events = watcher.poll();
    let elapsed = started.elapsed();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, WatchEvent::Changed(_))),
        "a change must be reported: {events:?}"
    );
    let reported = events.iter().find_map(diagnostics);
    assert!(
        reported.is_some(),
        "a type error must give diagnostics: {events:?}"
    );
    assert!(
        !reported.expect("diagnostics").is_empty(),
        "the diagnostic list must not be empty"
    );
    assert!(
        elapsed.as_millis() < 50,
        "edit-to-diagnostic must be fast, took {elapsed:?}"
    );

    // Fix the file; the next poll must say so.
    write(&dir, "main.ya", "fn main() {}\n");
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, WatchEvent::Success { .. })),
        "a fix must report success: {events:?}"
    );
}

#[test]
fn watching_one_file_ignores_its_neighbours() {
    let dir = work_dir("watch-single");
    write(&dir, "main.ya", "fn main() {}\n");
    write(&dir, "other.ya", "fn main() {}\n");
    let only: PathBuf = dir.join("main.ya");

    let mut watcher = Watcher::new(&dir, Some(&only)).expect("the watcher must start");
    write(&dir, "other.ya", "fn main() { const int x = \"bad\"; }\n");
    assert!(
        watcher.poll().is_empty(),
        "a file outside the watch set must be ignored"
    );

    write(&dir, "main.ya", "fn main() {}\n");
    let events = watcher.poll();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, WatchEvent::Changed(_))),
        "the watched file must be reported: {events:?}"
    );
}
