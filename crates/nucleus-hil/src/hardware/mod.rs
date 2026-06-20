//! The hardware HIL backend: flash a real board (`st-flash`), observe via
//! OpenOCD's SWO stream (decoded by [`itm`]) and gdbserver (memory reads via
//! [`crate::gdbstub`]).

pub mod itm;

use std::process::Command;
use std::time::{Duration, Instant};

use nucleus_compiler::CheckReport;
use nucleus_db::Port;

use crate::backend::{
    Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, RunTiming,
};
use crate::backend::{RunStatus, Sample};
use crate::preflight;

/// Whether `tool` can be spawned (i.e. exists on `PATH`). Mirrors
/// `nucleus-cli/src/firmware.rs`'s helper of the same name — duplicated here
/// rather than shared across crates; promoting it to its own crate isn't
/// worth the dependency-graph churn for one ~10-line helper.
fn tool_available(tool: &str) -> bool {
    use std::io::ErrorKind;
    match Command::new(tool).arg("--version").output() {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

/// Lightweight ST-Link probe detection, used to decide skip-vs-run without
/// ever touching firmware on a board that isn't there.
///
/// `st-info --probe` exits 0 even when it finds zero programmers (it prints
/// `Found 0 stlink programmers`), so the exit code alone can't distinguish
/// "no board" from "found one" — check the reported count in stdout instead.
fn board_detected() -> bool {
    if !tool_available("st-info") {
        return false;
    }
    let Ok(out) = Command::new("st-info").arg("--probe").output() else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    !stdout.contains("Found 0 stlink programmers")
}

#[derive(Default)]
pub struct HardwareBackend {
    started: bool,
    skip_reason: Option<String>,
    start_time: Option<Instant>,
    log: Vec<String>,
    traces: Vec<ItmEvent>,
}

impl Backend for HardwareBackend {
    fn name(&self) -> BackendKind {
        BackendKind::Hardware
    }

    fn start(
        &mut self,
        _firmware: &FirmwareArtifact,
        check_report: &CheckReport,
    ) -> Result<(), HilError> {
        preflight::gate(check_report)?;

        if !tool_available("st-flash") {
            self.skip_reason = Some("st-flash not found on PATH".to_string());
            return Ok(());
        }
        if !board_detected() {
            self.skip_reason = Some("no ST-Link board detected".to_string());
            return Ok(());
        }

        self.start_time = Some(Instant::now());
        self.started = true;
        // Real flashing + OpenOCD/SWO wiring lands with the QEMU leg's e2e
        // test (TDD step 11/12) — this skeleton proves the gate + skip path.
        Ok(())
    }

    fn pin(&mut self, _port: Port, _pin_num: u8) -> Result<bool, HilError> {
        Err(HilError::NotObservable {
            peripheral: "GPIO (hardware backend not yet wired)".to_string(),
        })
    }

    fn register(&mut self, peripheral: &str, _offset: u32) -> Result<u32, HilError> {
        Err(HilError::NotObservable {
            peripheral: peripheral.to_string(),
        })
    }

    fn await_itm_event(&mut self, _timeout: Duration) -> Result<Option<ItmEvent>, HilError> {
        Ok(self.traces.pop())
    }

    fn sample(&mut self, _duration: Duration) -> Result<Sample, HilError> {
        Ok(Sample {
            readings: Vec::new(),
        })
    }

    fn finish(&mut self) -> RunResult {
        let status = match &self.skip_reason {
            Some(reason) => RunStatus::Skipped {
                reason: reason.clone(),
            },
            None if self.started => RunStatus::Completed,
            None => RunStatus::Skipped {
                reason: "start() was never called".to_string(),
            },
        };
        let total = self
            .start_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);
        RunResult {
            backend: BackendKind::Hardware,
            status,
            log: std::mem::take(&mut self.log),
            traces: std::mem::take(&mut self.traces),
            timing: RunTiming { total },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_compiler::check;

    fn clean_report() -> CheckReport {
        check("").expect("empty toml parses")
    }

    fn conflicting_report() -> CheckReport {
        check(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"

[peripherals.tim2]
channel1 = "PA5"
"#,
        )
        .expect("valid toml")
    }

    #[test]
    fn rejects_conflicting_config_before_touching_anything() {
        let mut backend = HardwareBackend::default();
        let firmware = FirmwareArtifact {
            elf: "unused.elf".into(),
            bin: "unused.bin".into(),
        };
        let result = backend.start(&firmware, &conflicting_report());
        assert!(matches!(result, Err(HilError::Preflight(_))));
        assert!(!backend.started);
    }

    #[test]
    fn skips_rather_than_fails_when_no_board_present() {
        // This test machine has no st-flash/board attached, which is exactly
        // the degradation path we're proving.
        let mut backend = HardwareBackend::default();
        let firmware = FirmwareArtifact {
            elf: "unused.elf".into(),
            bin: "unused.bin".into(),
        };
        backend.start(&firmware, &clean_report()).unwrap();
        let result = backend.finish();
        assert!(matches!(result.status, RunStatus::Skipped { .. }));
    }
}
