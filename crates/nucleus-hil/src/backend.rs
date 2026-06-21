//! The shared vocabulary every HIL backend speaks: the [`Backend`] trait plus
//! the value types passed across it. No behavior lives here — just shapes.

use std::path::PathBuf;
use std::time::Duration;

use nucleus_compiler::{CheckReport, Conflict};

/// Which concrete backend produced a [`RunResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Qemu,
    Hardware,
}

/// Build artifacts a backend needs to start a run. QEMU needs the ELF (symbols
/// for the GDB stub, `-kernel` load); hardware flashing uses the raw `.bin`.
#[derive(Debug, Clone)]
pub struct FirmwareArtifact {
    pub elf: PathBuf,
    pub bin: PathBuf,
}

/// One decoded ITM packet surfaced to a test, already translated from
/// [`nucleus_itm::Packet`] into the shape both backends agree on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItmEvent {
    pub port: u8,
    pub data: Vec<u8>,
}

/// What a [`Sample`]'s `bool` readings actually mean. The two backends don't
/// observe the same thing here — `QemuBackend` can't see GPIO (see its module
/// doc comment) and falls back to polling whether TIM2's counter changed,
/// while `HardwareBackend` reads the real pin — so a bare `Vec<(Duration,
/// bool)>` with no tag would let M6/M10 silently compare apples to oranges.
/// Carrying the target explicitly forces call sites to either match on it or
/// confirm both sides used the same one before treating two `Sample`s as
/// comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleTarget {
    /// `readings` are the pin's actual level.
    Pin { port: nucleus_db::Port, pin_num: u8 },
    /// `readings` are whether `register(peripheral, offset)` changed value
    /// since the previous poll, not the value itself.
    RegisterChanged {
        peripheral: &'static str,
        offset: u32,
    },
}

/// A reading taken at one point in time over a window, as returned by
/// [`Backend::sample`]. See [`SampleTarget`] for what the `bool` means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub target: SampleTarget,
    pub readings: Vec<(Duration, bool)>,
}

/// Everything that can go wrong talking to a backend. Never a panic — garbage
/// on the wire or a missing tool always becomes one of these variants.
#[derive(Debug)]
pub enum HilError {
    /// The pre-flight gate rejected the config; `start()` never touched the
    /// emulator or hardware.
    Preflight(Vec<Conflict>),
    /// The backend's required tool (`qemu-system-arm`, `st-flash`, ...) isn't
    /// on `PATH`. Callers should usually treat this as `RunStatus::Skipped`,
    /// not `Failed`.
    ToolMissing(String),
    Io(std::io::Error),
    /// A malformed reply on the wire (e.g. a truncated GDB-RSP packet).
    Protocol(String),
    /// The backend doesn't model this peripheral (e.g. a non-GPIO block on
    /// QEMU's `netduinoplus2` machine).
    NotObservable {
        peripheral: String,
    },
}

impl std::fmt::Display for HilError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HilError::Preflight(conflicts) => {
                write!(f, "config has {} unresolved conflict(s)", conflicts.len())
            }
            HilError::ToolMissing(tool) => write!(f, "required tool not found: {tool}"),
            HilError::Io(err) => write!(f, "I/O error: {err}"),
            HilError::Protocol(msg) => write!(f, "protocol error: {msg}"),
            HilError::NotObservable { peripheral } => {
                write!(f, "peripheral not observable on this backend: {peripheral}")
            }
        }
    }
}

impl std::error::Error for HilError {}

impl From<std::io::Error> for HilError {
    fn from(err: std::io::Error) -> Self {
        HilError::Io(err)
    }
}

/// The outcome of one backend's run, independent of whether it actually ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Completed,
    /// The backend's tool/hardware wasn't available. Not a failure — the
    /// other backend may still have run.
    Skipped {
        reason: String,
    },
    Failed {
        error: String,
    },
}

/// Wall-clock timing for one run, useful for CI reporting later (M9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunTiming {
    pub total: Duration,
}

/// What one backend produced for one test run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub backend: BackendKind,
    pub status: RunStatus,
    pub log: Vec<String>,
    pub traces: Vec<ItmEvent>,
    pub timing: RunTiming,
}

/// The interface both the QEMU and hardware backends implement. Every method
/// here is observation, not assertion — `[[test]]` assertions are M6.
pub trait Backend {
    fn name(&self) -> BackendKind;

    /// Boot (QEMU) or flash (hardware) `firmware` and block until ready to
    /// observe. Implementations MUST call [`crate::preflight::gate`] on
    /// `check_report` as their first action and return
    /// `Err(HilError::Preflight(..))` without spawning anything if it fails.
    fn start(
        &mut self,
        firmware: &FirmwareArtifact,
        check_report: &CheckReport,
    ) -> Result<(), HilError>;

    fn pin(&mut self, port: nucleus_db::Port, pin_num: u8) -> Result<bool, HilError>;

    fn register(&mut self, peripheral: &str, offset: u32) -> Result<u32, HilError>;

    /// Read one 32-bit little-endian word at absolute address `addr`.
    fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError>;

    /// Write one 32-bit little-endian word `value` to absolute address `addr`.
    fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError>;

    fn await_itm_event(&mut self, timeout: Duration) -> Result<Option<ItmEvent>, HilError>;

    fn sample(&mut self, duration: Duration) -> Result<Sample, HilError>;

    /// Tear down and collect the accumulated result. Always succeeds — any
    /// failure already recorded becomes `RunStatus::Failed`.
    fn finish(&mut self) -> RunResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_result_constructs() {
        let result = RunResult {
            backend: BackendKind::Qemu,
            status: RunStatus::Completed,
            log: vec!["boot ok".to_string()],
            traces: vec![ItmEvent {
                port: 0,
                data: b"hi".to_vec(),
            }],
            timing: RunTiming {
                total: Duration::from_millis(10),
            },
        };
        assert_eq!(result.backend, BackendKind::Qemu);
        assert_eq!(result.status, RunStatus::Completed);
    }

    #[test]
    fn hil_error_displays_without_panicking() {
        let errs = vec![
            HilError::Preflight(vec![]),
            HilError::ToolMissing("qemu-system-arm".into()),
            HilError::Protocol("truncated reply".into()),
            HilError::NotObservable {
                peripheral: "SAI1".into(),
            },
        ];
        for err in errs {
            let _ = err.to_string();
        }
    }
}
