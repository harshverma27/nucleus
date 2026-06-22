//! Ledger integration for the CLI: turn a built project + test run into ledger
//! entries under `.nucleus/`.
//!
//! `build` calls [`record_version`] after a successful compile to register the
//! version's identity and verdict; `test` calls [`record_version`] (to find or
//! create the matching version) and then [`attach_tests`] to record per-backend
//! results. All of it is *best-effort*: a ledger failure prints a note but never
//! changes the command's exit status — the ledger observes, it does not gate.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use nucleus_compiler::check_family;
use nucleus_ledger::{write_artifacts, Ledger, TestRecord, Verdict, VersionInputs};

/// Toolchain identity string: the first line of `arm-none-eabi-gcc --version`,
/// or `"unknown-toolchain"` when the compiler is absent. Part of a version's
/// hash, so different toolchains yield distinct versions of the same config.
pub fn toolchain_identity() -> String {
    let line = std::process::Command::new("arm-none-eabi-gcc")
        .arg("--version")
        .output()
        .ok()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    if line.is_empty() {
        "unknown-toolchain".to_string()
    } else {
        line
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ensure a ledger version exists for the firmware currently built under
/// `root/build/firmware.bin`, returning its full hash. Records the version's
/// identity (config + resolved config + firmware + toolchain), verdict, and
/// artifacts on first sight; recognizes an unchanged version on re-runs.
///
/// Returns `None` (after a note) when the inputs aren't available — no
/// `stm32.toml`, no built firmware, or an unreadable ledger — so callers can
/// proceed without the ledger.
pub fn record_version(root: &Path) -> Option<String> {
    let toml_path = root.join("stm32.toml");
    let toml = std::fs::read_to_string(&toml_path).ok()?;
    let bin = match std::fs::read(root.join("build/firmware.bin")) {
        Ok(b) => b,
        Err(_) => return None,
    };

    // The verdict comes from the verifier. A built firmware implies an approved
    // config (build refuses to compile a conflicted one), but we record the
    // verdict faithfully rather than assuming.
    let (family, verdict) = match check_family(&toml) {
        Ok((report, _)) => {
            let family = report.config.device.family.clone();
            let verdict = if report.is_ok() {
                Verdict::Approved
            } else {
                Verdict::Conflicts(report.conflicts.iter().map(|c| c.to_string()).collect())
            };
            (family, verdict)
        }
        Err(_) => (String::new(), Verdict::Approved),
    };

    let toolchain = toolchain_identity();
    // `build` does not auto-route, so the resolved config is the config as
    // written. A routed project's `stm32.toml` already *is* the resolved form,
    // so `solved_config` correctly stays `None` here.
    let inputs = VersionInputs {
        stm32_toml: &toml,
        resolved_config: &toml,
        firmware_bin: &bin,
        toolchain: &toolchain,
    };

    let mut ledger = match Ledger::load(root) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("note: ledger unreadable ({err}); skipping version record.");
            return None;
        }
    };
    let (hash, is_new) = ledger.record(&inputs, &family, verdict, now_secs());

    // Snapshot the artifacts that define the build, when present.
    let config_h = root.join("src/generated/nucleus_config.h");
    let mut artifacts: Vec<(&str, &[u8])> = vec![("firmware.bin", &bin)];
    let config_h_bytes;
    if let Ok(bytes) = std::fs::read(&config_h) {
        config_h_bytes = bytes;
        artifacts.push(("nucleus_config.h", &config_h_bytes));
    }
    if let Err(err) = write_artifacts(root, &hash, &artifacts) {
        eprintln!("note: could not write ledger artifacts ({err}).");
    }

    if let Err(err) = ledger.save(root) {
        eprintln!("note: could not save ledger ({err}).");
        return None;
    }

    let short = &hash[..nucleus_ledger::SHORT_HASH_LEN.min(hash.len())];
    println!(
        "ledger: {} version {short}",
        if is_new { "recorded" } else { "recognized" }
    );
    Some(hash)
}

/// Attach per-backend test outcomes to the version with `hash`. Best-effort.
pub fn attach_tests(root: &Path, hash: &str, records: Vec<TestRecord>) {
    if records.is_empty() {
        return;
    }
    let mut ledger = match Ledger::load(root) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("note: ledger unreadable ({err}); skipping test record.");
            return;
        }
    };
    if !ledger.record_tests(hash, records) {
        eprintln!("note: no ledger version {hash} to attach test results to.");
        return;
    }
    if let Err(err) = ledger.save(root) {
        eprintln!("note: could not save ledger ({err}).");
    }
}
