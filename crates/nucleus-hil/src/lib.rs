//! Nucleus's dual-backend HIL substrate (v2 milestone M5).
//!
//! One [`backend::Backend`] trait, two implementations: [`qemu::QemuBackend`]
//! (emulated, runs everywhere) and [`hardware::HardwareBackend`] (real board
//! via SWD/ITM). Both expose the same observation API and never run a config
//! the verifier (`nucleus-compiler`) flagged — see [`preflight::gate`].
//!
//! `nucleus test` and `[[test]]` parsing are out of scope here (M6/M7); this
//! crate is the substrate those milestones build on.

pub mod backend;
pub mod gdbstub;
pub mod gpio_map;
pub mod hardware;
pub mod preflight;
pub mod qemu;

pub use backend::{
    Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, RunStatus, RunTiming,
    Sample,
};

/// Runs `firmware` on both backends and always returns one [`RunResult`] per
/// backend — missing tools/hardware degrade to `RunStatus::Skipped` inside
/// each backend's own `start()`; a `start()` error (preflight rejection, or a
/// genuine spawn failure) becomes `RunStatus::Failed` here instead of being
/// dropped, since that's a real problem the caller needs to see.
pub fn run_all(
    firmware: &FirmwareArtifact,
    report: &nucleus_compiler::CheckReport,
) -> Vec<RunResult> {
    vec![
        run_one(qemu::QemuBackend::default(), firmware, report),
        run_one(hardware::HardwareBackend::default(), firmware, report),
    ]
}

fn run_one(
    mut backend: impl Backend,
    firmware: &FirmwareArtifact,
    report: &nucleus_compiler::CheckReport,
) -> RunResult {
    let kind = backend.name();
    match backend.start(firmware, report) {
        Ok(()) => backend.finish(),
        Err(err) => RunResult {
            backend: kind,
            status: RunStatus::Failed {
                error: err.to_string(),
            },
            log: Vec::new(),
            traces: Vec::new(),
            timing: RunTiming::default(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_compiler::check;

    #[test]
    fn run_all_returns_one_result_per_backend() {
        let firmware = FirmwareArtifact {
            elf: "unused.elf".into(),
            bin: "unused.bin".into(),
        };
        let report = check("").expect("empty config parses");
        let results = run_all(&firmware, &report);
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.backend == BackendKind::Qemu));
        assert!(results.iter().any(|r| r.backend == BackendKind::Hardware));
    }

    #[test]
    fn run_all_fails_both_backends_on_a_rejected_config() {
        let firmware = FirmwareArtifact {
            elf: "unused.elf".into(),
            bin: "unused.bin".into(),
        };
        let report = check(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"

[peripherals.tim2]
channel1 = "PA5"
"#,
        )
        .expect("valid toml");
        let results = run_all(&firmware, &report);
        assert_eq!(results.len(), 2);
        for result in results {
            assert!(matches!(result.status, RunStatus::Failed { .. }));
        }
    }
}
