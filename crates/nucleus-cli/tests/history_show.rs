//! CLI-level coverage for the M8 `nucleus history` / `nucleus show` verbs,
//! driving the real built binary against a hand-seeded `.nucleus/` ledger so the
//! verbs are exercised without needing the cross toolchain or a board.

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

/// Seed `.nucleus/versions.json` with two versions, one with mixed results.
fn seed(root: &Path) {
    let dir = root.join(".nucleus");
    std::fs::create_dir_all(&dir).unwrap();
    let json = r#"{
      "versions": [
        {
          "hash": "aaaa111100000000000000000000000000000000000000000000000000000000",
          "timestamp": 1735689600,
          "family": "STM32F446RE",
          "toolchain": "arm-none-eabi-gcc 13.2.0",
          "stm32_toml": "[device]\nfamily = \"STM32F446RE\"\n",
          "solved_config": null,
          "verdict": { "verdict": "approved" },
          "tests": [
            { "name": "echo", "backend": "qemu", "status": "pass", "detail": "ok" },
            { "name": "echo", "backend": "hardware", "status": "fail", "detail": "no reply" }
          ]
        },
        {
          "hash": "bbbb222200000000000000000000000000000000000000000000000000000000",
          "timestamp": 1782135909,
          "family": "STM32F446RE",
          "toolchain": "arm-none-eabi-gcc 13.2.0",
          "stm32_toml": "[device]\nfamily = \"STM32F446RE\"\n",
          "solved_config": null,
          "verdict": { "verdict": "conflicts", "conflicts": ["TIM2 clock too fast"] },
          "tests": []
        }
      ]
    }"#;
    std::fs::write(dir.join("versions.json"), json).unwrap();
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
fn history_lists_versions_with_per_backend_results() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["history"], &root);
    assert!(ok, "history should exit 0: {out}");
    assert!(out.contains("aaaa111"), "short hash of v1: {out}");
    assert!(out.contains("bbbb222"), "short hash of v2: {out}");
    assert!(out.contains("qemu 1/1"), "qemu pass count: {out}");
    assert!(out.contains("hardware 0/1"), "hardware fail count: {out}");
    assert!(out.contains("approved"), "v1 verdict: {out}");
    assert!(out.contains("conflict"), "v2 verdict: {out}");
    // Timestamp formatting.
    assert!(out.contains("2025-01-01"), "v1 date: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn show_resolves_short_hash_and_prints_detail() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["show", "aaaa111"], &root);
    assert!(ok, "show should exit 0: {out}");
    assert!(out.contains("aaaa1111"), "full/short hash: {out}");
    assert!(out.contains("PASS echo [qemu]"), "qemu result line: {out}");
    assert!(
        out.contains("FAIL echo [hardware]"),
        "hardware result line: {out}"
    );
    assert!(out.contains("approved"), "verdict: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn show_unknown_hash_fails() {
    let root = tempdir();
    seed(&root);

    let (ok, out) = run(&["show", "deadbeef"], &root);
    assert!(!ok, "unknown hash must exit non-zero");
    assert!(out.contains("no version matches"), "error message: {out}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn history_empty_when_no_ledger() {
    let root = tempdir();
    let (ok, out) = run(&["history"], &root);
    assert!(ok);
    assert!(out.contains("no versions recorded"), "{out}");
    std::fs::remove_dir_all(&root).ok();
}
