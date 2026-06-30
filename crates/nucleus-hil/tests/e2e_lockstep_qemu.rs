//! M10's exit-criterion proof for the QEMU leg: `lockstep::collect` against
//! a real `qemu-system-arm` run produces a non-empty `ObservationTrace`, and
//! comparing two independent collections of the same firmware agrees
//! end-to-end. No live two-backend (QEMU-vs-silicon) divergence fixture —
//! see `docs/superpowers/specs/2026-06-23-m10-lockstep-design.md` for why
//! (QEMU has no GPIO model on `netduinoplus2`; real hardware SWO/GDB is
//! known-flaky per the M5/M7 findings, so a deterministic CI fixture can't
//! depend on a live mismatch happening on demand).

use std::path::PathBuf;
use std::time::Duration;

use nucleus_compiler::check;
use nucleus_hil::backend::{Backend, FirmwareArtifact};
use nucleus_hil::lockstep::{self, DivergenceReport};
use nucleus_hil::qemu::QemuBackend;
use nucleus_trace::translate::VariableMap;

fn fixture_elf() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/blink_itm/blink_itm_qemu.elf")
}

fn qemu_available() -> bool {
    std::process::Command::new("qemu-system-arm")
        .arg("--version")
        .output()
        .is_ok()
}

/// `blink_itm` emits two single-byte SWIT packets (`'O'` then `'K'`) on
/// boot. `collect()` should capture both as checkpoints with `decoded:
/// None` (no `[trace.variables]` entry names port 0 — it's the log stream).
#[test]
fn collect_captures_real_qemu_itm_boot_log() {
    if !qemu_available() {
        eprintln!("skipping: qemu-system-arm not installed");
        return;
    }

    let firmware = FirmwareArtifact {
        elf: fixture_elf(),
        bin: PathBuf::new(),
    };
    let report = check("").expect("empty config parses");
    let vars = VariableMap::new();

    let mut backend = QemuBackend::default();
    backend.start(&firmware, &report).expect("qemu boots");

    let trace = lockstep::collect(
        &mut backend,
        &vars,
        Duration::from_secs(3),
        Duration::from_secs(5),
    )
    .expect("collection against a healthy qemu backend succeeds");

    assert!(
        !trace.checkpoints.is_empty(),
        "expected at least one ITM checkpoint from blink_itm's boot log"
    );
    assert_eq!(trace.checkpoints[0].itm_event.port, 0);
    assert_eq!(trace.checkpoints[0].itm_event.data, vec![b'O']);
    assert_eq!(trace.checkpoints[0].decoded, None);

    let _ = backend.finish();
}

/// Two independent collections of the same firmware should agree
/// end-to-end — the certificate path the issue calls out ("agreement to
/// end" for clean firmware with no timing dependencies).
#[test]
fn comparing_two_collections_of_the_same_firmware_agrees() {
    if !qemu_available() {
        eprintln!("skipping: qemu-system-arm not installed");
        return;
    }

    let firmware = FirmwareArtifact {
        elf: fixture_elf(),
        bin: PathBuf::new(),
    };
    let report = check("").expect("empty config parses");
    let vars = VariableMap::new();

    let mut backend_a = QemuBackend::default();
    backend_a.start(&firmware, &report).expect("qemu boots");
    let trace_a = lockstep::collect(
        &mut backend_a,
        &vars,
        Duration::from_secs(3),
        Duration::from_secs(5),
    )
    .expect("collection against a healthy qemu backend succeeds");
    let _ = backend_a.finish();

    let mut backend_b = QemuBackend::default();
    backend_b.start(&firmware, &report).expect("qemu boots");
    let trace_b = lockstep::collect(
        &mut backend_b,
        &vars,
        Duration::from_secs(3),
        Duration::from_secs(5),
    )
    .expect("collection against a healthy qemu backend succeeds");
    let _ = backend_b.finish();

    let report = lockstep::compare(&trace_a, &trace_b);
    match report {
        DivergenceReport::Agreement {
            checkpoints_compared,
        } => {
            assert!(checkpoints_compared > 0);
        }
        other => {
            panic!("expected Agreement comparing two runs of identical firmware, got {other:?}")
        }
    }
}
