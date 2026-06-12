//! End-to-end tests for the `nucleus` binary against repo fixtures.
//!
//! These exercise the Phase 2 exit-criterion directly: `nucleus check` must
//! exit non-zero on conflicts (so CI can gate on it) and zero on a clean file,
//! and the deliberate PA5-collision fixture must yield exactly one error.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn run_check(name: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nucleus"))
        .arg("check")
        .arg(fixture(name))
        .output()
        .expect("failed to run nucleus binary")
}

#[test]
fn clean_config_exits_zero() {
    let out = run_check("clean.toml");
    assert!(
        out.status.success(),
        "expected success, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn pa5_collision_exits_nonzero_with_exactly_one_error() {
    let out = run_check("pa5_collision.toml");
    assert!(!out.status.success(), "expected non-zero exit on collision");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let error_lines = stderr.lines().filter(|l| l.contains("error:")).count();
    assert_eq!(
        error_lines, 1,
        "expected exactly one error, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("PA5") && stderr.contains("pin collision"),
        "error should name the colliding pin, got:\n{stderr}"
    );
}

#[test]
fn missing_file_exits_nonzero() {
    let out = Command::new(env!("CARGO_BIN_EXE_nucleus"))
        .arg("check")
        .arg("definitely-does-not-exist.toml")
        .output()
        .expect("failed to run nucleus binary");
    assert!(!out.status.success());
}
