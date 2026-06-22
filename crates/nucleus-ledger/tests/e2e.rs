//! End-to-end ledger flow: several "commits" (distinct config+firmware), each
//! built and tested on both backends, then a re-run of an unchanged version.
//! Mirrors M8's exit criterion: history shows each version with per-backend
//! results, and re-running an unchanged version is recognized (no duplicate).

use std::path::PathBuf;

use nucleus_ledger::{Ledger, TestRecord, Verdict, VersionInputs};

fn tempdir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nucleus-ledger-e2e-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rec(name: &str, backend: &str, status: &str) -> TestRecord {
    TestRecord {
        name: name.into(),
        backend: backend.into(),
        status: status.into(),
        detail: format!("{name} on {backend}"),
    }
}

/// Simulate `nucleus build` + `nucleus test` for one project state.
fn build_and_test(
    root: &std::path::Path,
    toml: &str,
    bin: &[u8],
    ts: u64,
    results: Vec<TestRecord>,
) {
    let inputs = VersionInputs {
        stm32_toml: toml,
        resolved_config: toml,
        firmware_bin: bin,
        toolchain: "arm-none-eabi-gcc 13.2.0",
    };
    let mut ledger = Ledger::load(root).unwrap();
    let (hash, _) = ledger.record(&inputs, "STM32F446RE", Verdict::Approved, ts);
    ledger.record_tests(&hash, results);
    ledger.save(root).unwrap();
}

#[test]
fn three_commits_then_unchanged_rerun() {
    let root = tempdir();

    // Commit 1: passes everywhere.
    build_and_test(
        &root,
        "[device]\nfamily = \"STM32F446RE\"\n# v1",
        b"\x01firmware-one",
        1_000,
        vec![
            rec("blink", "qemu", "pass"),
            rec("blink", "hardware", "pass"),
        ],
    );

    // Commit 2: a regression on hardware only.
    build_and_test(
        &root,
        "[device]\nfamily = \"STM32F446RE\"\n# v2",
        b"\x02firmware-two",
        2_000,
        vec![
            rec("blink", "qemu", "pass"),
            rec("blink", "hardware", "fail"),
        ],
    );

    // Commit 3: pin assertion that QEMU can't observe (skip), hardware passes.
    build_and_test(
        &root,
        "[device]\nfamily = \"STM32F446RE\"\n# v3",
        b"\x03firmware-three",
        3_000,
        vec![
            rec("pa5_toggle", "qemu", "skip"),
            rec("pa5_toggle", "hardware", "pass"),
        ],
    );

    // Three distinct versions recorded, in order.
    let ledger = Ledger::load(&root).unwrap();
    assert_eq!(ledger.versions.len(), 3);

    // Per-backend summaries are what `nucleus history` would print.
    let s1 = Ledger::test_summary(&ledger.versions[0]);
    assert_eq!(s1["qemu"], (1, 1));
    assert_eq!(s1["hardware"], (1, 1));

    let s2 = Ledger::test_summary(&ledger.versions[1]);
    assert_eq!(s2["qemu"], (1, 1));
    assert_eq!(s2["hardware"], (0, 1)); // hardware regression

    let s3 = Ledger::test_summary(&ledger.versions[2]);
    assert_eq!(s3["qemu"], (0, 0)); // skip excluded from total
    assert_eq!(s3["hardware"], (1, 1));

    // Re-run commit 2 unchanged: recognized, not duplicated.
    let before = ledger.versions.len();
    build_and_test(
        &root,
        "[device]\nfamily = \"STM32F446RE\"\n# v2",
        b"\x02firmware-two",
        9_999, // later timestamp must not matter
        vec![
            rec("blink", "qemu", "pass"),
            rec("blink", "hardware", "pass"),
        ], // now fixed
    );
    let after = Ledger::load(&root).unwrap();
    assert_eq!(
        after.versions.len(),
        before,
        "unchanged re-run must not add a version"
    );

    // Identity (timestamp) preserved; only the test result updated in place.
    assert_eq!(after.versions[1].timestamp, 2_000);
    let s2_after = Ledger::test_summary(&after.versions[1]);
    assert_eq!(
        s2_after["hardware"],
        (1, 1),
        "re-run updates the result for the same version"
    );

    std::fs::remove_dir_all(&root).ok();
}
