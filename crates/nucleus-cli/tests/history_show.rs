//! CLI-level coverage for `nucleus history` / `nucleus show`, driving the real
//! built binary against a hand-seeded `tests/test_history.json` so the verbs are
//! exercised without needing the cross toolchain or a board.

use std::path::{Path, PathBuf};
use std::process::Command;

const NUCLEUS: &str = env!("CARGO_BIN_EXE_nucleus");

fn tempdir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nucleus-cli-hist-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Seed `tests/test_history.json` with two runs: run 1 mixed, run 2 all-pass.
fn seed(root: &Path) {
    let dir = root.join("tests");
    std::fs::create_dir_all(&dir).unwrap();
    let json = r#"{
      "runs": [
        {
          "timestamp": 1735689600,
          "tests": [
            { "name": "echo", "backend": "qemu", "status": "pass", "detail": "ok" },
            { "name": "echo", "backend": "hardware", "status": "fail", "detail": "no reply" },
            { "name": "pa5", "backend": "qemu", "status": "skip", "detail": "not observable" }
          ]
        },
        {
          "timestamp": 1782135909,
          "tests": [
            { "name": "echo", "backend": "qemu", "status": "pass", "detail": "ok" }
          ]
        }
      ]
    }"#;
    std::fs::write(dir.join("test_history.json"), json).unwrap();
}

fn run(args: &[&str], root: &Path) -> (bool, String) {
    let out = Command::new(NUCLEUS)
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), s)
}

#[test]
fn history_lists_runs_with_counts() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["history"], &root);
    assert!(ok, "history should exit 0: {out}");
    assert!(out.contains("#1"), "run 1 listed: {out}");
    assert!(out.contains("#2"), "run 2 listed: {out}");
    assert!(
        out.contains("1 passed, 1 failed, 1 skipped"),
        "run 1 counts: {out}"
    );
    assert!(out.contains("2025-01-01"), "run 1 date: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn history_empty_when_no_file() {
    let root = tempdir();
    let (ok, out) = run(&["history"], &root);
    assert!(ok);
    assert!(out.contains("no test runs recorded"), "{out}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn history_graph_emits_summary_json() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["history", "--graph"], &root);
    assert!(ok, "history --graph should exit 0: {out}");
    assert!(
        out.contains("\"schema\": \"nucleus.history.v1\""),
        "schema: {out}"
    );
    assert!(out.contains("\"pass\": 1"), "pass count: {out}");
    assert!(out.contains("\"fail\": 1"), "fail count: {out}");
    assert!(out.contains("\"skip\": 1"), "skip count: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn history_graph_empty_is_valid_json() {
    let root = tempdir();
    let (ok, out) = run(&["history", "--graph"], &root);
    assert!(ok);
    assert!(out.contains("nucleus.history.v1"), "{out}");
    assert!(out.contains("\"runs\": []"), "{out}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn history_last_limits_table() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["history", "--last", "1"], &root);
    assert!(ok);
    assert!(out.contains("#2"), "latest run shown: {out}");
    assert!(!out.contains("#1 "), "older run dropped: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn show_defaults_to_latest_run() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["show"], &root);
    assert!(ok, "show should exit 0: {out}");
    assert!(out.contains("run      #2"), "latest run: {out}");
    assert!(out.contains("PASS echo [qemu]"), "result line: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn show_by_index_lists_all_results() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["show", "1"], &root);
    assert!(ok, "show 1 should exit 0: {out}");
    assert!(out.contains("run      #1"), "run 1: {out}");
    assert!(out.contains("PASS echo [qemu]"), "qemu pass: {out}");
    assert!(out.contains("FAIL echo [hardware]"), "hardware fail: {out}");
    assert!(out.contains("SKIP pa5 [qemu]"), "skip line: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn show_out_of_range_fails() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["show", "99"], &root);
    assert!(!ok, "out-of-range run must exit non-zero");
    assert!(out.contains("no run #99"), "error message: {out}");

    std::fs::remove_dir_all(&root).ok();
}
