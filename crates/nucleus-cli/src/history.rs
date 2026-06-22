//! Test-history integration for the CLI.
//!
//! `nucleus test` calls [`record_run`] once per invocation: it appends a [`Run`]
//! (timestamp + every assertion's result) to `tests/test_history.json`. This is
//! best-effort — a write failure prints a note but never changes the command's
//! exit status; the history observes, it does not gate.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use nucleus_history::{Run, TestEntry, TestHistory};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append one run's results to `root`'s test history. Creates
/// `tests/test_history.json` on the first run and appends on every run after.
/// No-op when `entries` is empty (nothing ran).
pub fn record_run(root: &Path, entries: Vec<TestEntry>) {
    if entries.is_empty() {
        return;
    }
    let mut history = match TestHistory::load(root) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("note: test history unreadable ({err}); skipping record.");
            return;
        }
    };
    history.push(Run {
        timestamp: now_secs(),
        tests: entries,
    });
    if let Err(err) = history.save(root) {
        eprintln!("note: could not save test history ({err}).");
        return;
    }
    println!(
        "history: recorded run #{} -> {}",
        history.runs.len(),
        TestHistory::path(root).display()
    );
}
