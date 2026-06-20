//! Nucleus's dual-backend HIL substrate (v2 milestone M5).
//!
//! One [`backend::Backend`] trait, two implementations: [`qemu::QemuBackend`]
//! (emulated, runs everywhere) and [`hardware::HardwareBackend`] (real board
//! via SWD/ITM). Both expose the same observation API and never run a config
//! the verifier (`nucleus-compiler`) flagged — see [`preflight::gate`].
//!
//! `nucleus test` and `[[test]]` parsing are out of scope here (M6/M7); this
//! crate is the substrate those milestones build on.

pub mod assert;
pub mod backend;
pub mod gdbstub;
pub mod gpio_map;
pub mod hardware;
pub mod preflight;
pub mod qemu;

/// Ask the OS for a currently-free TCP port by binding `:0` and reading back
/// what it picked, then immediately dropping the listener to free it for the
/// real client (QEMU's `-gdb`, or OpenOCD). Avoids the hardcoded-port
/// collisions that broke concurrent backend instances in the same process
/// (two `QemuBackend`s booting at once both reaching for `:1234`, etc).
/// There's an inherent TOCTOU gap between the listener dropping and the real
/// process binding — acceptable here since nothing else on a CI/dev box is
/// racing for the same ephemeral port at the same instant.
pub(crate) fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

pub use assert::{TestOutcome, TestStatus};
pub use backend::{
    Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, RunStatus, RunTiming,
    Sample, SampleTarget,
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

/// Does `select` apply to `kind`? `BackendSelect::Both` always matches.
fn backend_selected(select: nucleus_compiler::BackendSelect, kind: BackendKind) -> bool {
    use nucleus_compiler::BackendSelect;
    match select {
        BackendSelect::Both => true,
        BackendSelect::Qemu => kind == BackendKind::Qemu,
        BackendSelect::Hardware => kind == BackendKind::Hardware,
    }
}

/// Run every test in `plan` against `backend`, filtering by each
/// [`nucleus_compiler::BackendSelect`] against `backend.name()` — a test
/// whose `backend` field doesn't select this backend becomes
/// `TestStatus::Skipped` without ever touching the backend (cheap, and keeps
/// a "hardware-only" test from spuriously running on QEMU).
pub fn run_tests(
    backend: &mut dyn Backend,
    plan: &[nucleus_compiler::CompiledTest],
) -> Vec<TestOutcome> {
    let kind = backend.name();
    plan.iter()
        .map(|test| {
            if backend_selected(test.backend, kind) {
                assert::run(backend, test)
            } else {
                TestOutcome {
                    name: test.name.clone(),
                    status: TestStatus::Skipped,
                    detail: "not selected for this backend".to_string(),
                }
            }
        })
        .collect()
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

    /// A minimal fake `Backend` for `run_tests` filtering tests — only
    /// `name()` matters for skip/run decisions, but every other method
    /// increments a call counter so a test can prove a skipped test never
    /// touches the backend.
    struct CountingBackend {
        kind: BackendKind,
        calls: std::cell::Cell<u32>,
    }

    impl Backend for CountingBackend {
        fn name(&self) -> BackendKind {
            self.kind
        }

        fn start(
            &mut self,
            _firmware: &FirmwareArtifact,
            _check_report: &nucleus_compiler::CheckReport,
        ) -> Result<(), HilError> {
            Ok(())
        }

        fn pin(&mut self, _port: nucleus_db::Port, _pin_num: u8) -> Result<bool, HilError> {
            self.calls.set(self.calls.get() + 1);
            Ok(true)
        }

        fn register(&mut self, _peripheral: &str, _offset: u32) -> Result<u32, HilError> {
            self.calls.set(self.calls.get() + 1);
            Ok(0)
        }

        fn await_itm_event(
            &mut self,
            _timeout: std::time::Duration,
        ) -> Result<Option<ItmEvent>, HilError> {
            self.calls.set(self.calls.get() + 1);
            Ok(None)
        }

        fn sample(&mut self, _duration: std::time::Duration) -> Result<Sample, HilError> {
            self.calls.set(self.calls.get() + 1);
            Ok(Sample {
                target: SampleTarget::Pin {
                    port: nucleus_db::Port::A,
                    pin_num: 5,
                },
                readings: vec![],
            })
        }

        fn finish(&mut self) -> RunResult {
            RunResult {
                backend: self.kind,
                status: RunStatus::Completed,
                log: vec![],
                traces: vec![],
                timing: RunTiming::default(),
            }
        }
    }

    fn compiled_test(
        name: &str,
        select: nucleus_compiler::BackendSelect,
    ) -> nucleus_compiler::CompiledTest {
        nucleus_compiler::CompiledTest {
            name: name.to_string(),
            assertion: nucleus_compiler::Assertion::PinState {
                pin: "PA5".to_string(),
                level: true,
                within: std::time::Duration::from_millis(5),
            },
            timeout: std::time::Duration::from_millis(50),
            backend: select,
        }
    }

    #[test]
    fn run_tests_runs_a_both_selected_test_regardless_of_backend_name() {
        let mut backend = CountingBackend {
            kind: BackendKind::Hardware,
            calls: std::cell::Cell::new(0),
        };
        let plan = vec![compiled_test("both", nucleus_compiler::BackendSelect::Both)];
        let outcomes = run_tests(&mut backend, &plan);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, TestStatus::Passed);
        assert!(
            backend.calls.get() > 0,
            "Both-selected test must touch the backend"
        );
    }

    #[test]
    fn run_tests_skips_a_non_matching_backend_select_without_touching_backend() {
        let mut backend = CountingBackend {
            kind: BackendKind::Hardware,
            calls: std::cell::Cell::new(0),
        };
        let plan = vec![compiled_test(
            "qemu-only",
            nucleus_compiler::BackendSelect::Qemu,
        )];
        let outcomes = run_tests(&mut backend, &plan);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, TestStatus::Skipped);
        assert_eq!(
            backend.calls.get(),
            0,
            "skipped test must never call into the backend"
        );
    }
}
