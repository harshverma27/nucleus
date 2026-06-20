//! M5's exit-criterion proof: one `Backend::start()` boots real firmware in
//! real `qemu-system-arm`, observes a changing register + an ITM log, and
//! returns a `RunResult`. Skips (doesn't fail) if QEMU isn't installed.
//!
//! Observes TIM2's free-running counter, not a GPIO pin — GPIO turned out to
//! be an `unimplemented_device` stub on QEMU's `netduinoplus2` machine (no
//! real model, confirmed via `-d unimp`), so it can't be read back. See
//! `src/qemu/mod.rs`'s module doc comment for the full finding; the hardware
//! backend still observes real GPIO, this gap is QEMU-only.

use std::path::PathBuf;
use std::time::Duration;

use nucleus_compiler::{check, test_plan};
use nucleus_hil::assert;
use nucleus_hil::backend::{Backend, FirmwareArtifact, RunStatus};
use nucleus_hil::qemu::QemuBackend;
use nucleus_hil::TestStatus;

fn fixture_elf() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/blink_itm/blink_itm_qemu.elf")
}

fn qemu_available() -> bool {
    std::process::Command::new("qemu-system-arm")
        .arg("--version")
        .output()
        .is_ok()
}

#[test]
fn blink_itm_advances_tim2_and_emits_itm_log_on_qemu() {
    if !qemu_available() {
        eprintln!("skipping: qemu-system-arm not installed");
        return;
    }

    let firmware = FirmwareArtifact {
        elf: fixture_elf(),
        bin: PathBuf::new(), // unused by the QEMU backend, which loads the ELF
    };
    let report = check("").expect("empty config parses");

    let mut backend = QemuBackend::default();
    backend.start(&firmware, &report).expect("qemu boots");

    // blink_itm emits two single-byte SWIT packets ('O' then 'K'); the
    // observation API surfaces one ItmEvent per packet, so the first await
    // sees 'O'.
    let log_event = backend
        .await_itm_event(Duration::from_secs(3))
        .expect("itm read")
        .expect("blink_itm emits an ITM log on boot");
    assert_eq!(log_event.port, 0);
    assert_eq!(log_event.data, vec![b'O']);

    // GPIO isn't observable on this QEMU machine (see module doc comment),
    // so the substrate's "observe state changing over a window" proof runs
    // against TIM2's counter instead.
    let pin_result = backend.pin(nucleus_db::Port::A, 5);
    assert!(matches!(
        pin_result,
        Err(nucleus_hil::backend::HilError::NotObservable { .. })
    ));

    let sample = backend
        .sample(Duration::from_millis(200))
        .expect("tim2 sampling");
    let changed = sample.readings.iter().any(|(_, changed)| *changed);
    assert!(
        changed,
        "expected TIM2's counter to change across the sample window, got {:?}",
        sample.readings
    );

    let result = backend.finish();
    assert_eq!(result.status, RunStatus::Completed);
}

/// M6's exit-criterion proof: a `[[test]]` block, parsed end-to-end through
/// `nucleus_compiler::test_plan` (TOML -> `CompiledTest`), passes when run
/// against the real `blink_itm` firmware's boot ITM log via
/// `nucleus_hil::assert::run`. `blink_itm` emits `'O'` then `'K'` as two
/// single-byte SWIT packets on boot (see `blink_itm_advances_tim2_and_emits_itm_log_on_qemu`
/// above), so a pattern of `"O"` matches the first observed event.
#[test]
fn compiled_test_plan_passes_against_real_qemu_itm_boot_log() {
    if !qemu_available() {
        eprintln!("skipping: qemu-system-arm not installed");
        return;
    }

    let plan = test_plan(
        r#"
[[test]]
name = "boot_log"
assertion = "trace event \"O\" within 3000ms"
"#,
    )
    .expect("parses")
    .expect("no conflicts");
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].name, "boot_log");

    let firmware = FirmwareArtifact {
        elf: fixture_elf(),
        bin: PathBuf::new(),
    };
    let report = check("").expect("empty config parses");

    let mut backend = QemuBackend::default();
    backend.start(&firmware, &report).expect("qemu boots");

    let outcome = assert::run(&mut backend, &plan[0]);
    assert_eq!(outcome.status, TestStatus::Passed, "{}", outcome.detail);

    let result = backend.finish();
    assert_eq!(result.status, RunStatus::Completed);
}

/// Same pipeline, but the pattern never appears in `blink_itm`'s boot log
/// (which only ever emits `'O'` then `'K'`), so the assertion must fail
/// rather than time out silently or pass by accident.
#[test]
fn compiled_test_plan_fails_on_an_itm_pattern_that_never_appears() {
    if !qemu_available() {
        eprintln!("skipping: qemu-system-arm not installed");
        return;
    }

    let plan = test_plan(
        r#"
[[test]]
name = "never_happens"
assertion = "trace event \"never_happens\" within 500ms"
"#,
    )
    .expect("parses")
    .expect("no conflicts");
    assert_eq!(plan.len(), 1);

    let firmware = FirmwareArtifact {
        elf: fixture_elf(),
        bin: PathBuf::new(),
    };
    let report = check("").expect("empty config parses");

    let mut backend = QemuBackend::default();
    backend.start(&firmware, &report).expect("qemu boots");

    let outcome = assert::run(&mut backend, &plan[0]);
    assert_eq!(outcome.status, TestStatus::Failed, "{}", outcome.detail);

    let result = backend.finish();
    assert_eq!(result.status, RunStatus::Completed);
}
