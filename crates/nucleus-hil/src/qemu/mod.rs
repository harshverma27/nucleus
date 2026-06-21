//! The QEMU HIL backend: boot firmware in an emulated STM32F4 (the closest
//! upstream QEMU has — `netduinoplus2`, modeling an F405 — see crate docs for
//! the fidelity caveat), observe via the GDB-RSP stub QEMU exposes with
//! `-gdb`, and decode an ITM-encoded log shipped over a semihosting channel
//! (see [`itm`] — QEMU's STM32F4 model has no real TPIU/ITM block).
//!
//! **GPIO is not modeled either.** `info mtree` on `netduinoplus2` shows
//! GPIOA–I registered as `unimplemented_device` stubs (confirmed via
//! `-d unimp`: writes are silently discarded, reads always return 0) — only
//! `stm32f2xx_timer` (4 instances: TIM2–5), USART, SPI, ADC, RCC, SYSCFG, and
//! EXTI have real models. `pin()`/`register("GPIOx", ..)` therefore always
//! return [`HilError::NotObservable`] on this backend; `register`/`sample`
//! observe TIM2's free-running counter instead, which genuinely changes
//! state and is readable through the same GDB-RSP path. The hardware backend
//! still observes real GPIO — this gap is QEMU-only.

pub mod itm;

use std::collections::VecDeque;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nucleus_compiler::CheckReport;
use nucleus_db::Port;
use nucleus_itm::Decoder;

use crate::backend::{
    Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, RunStatus, RunTiming,
    Sample, SampleTarget,
};
use crate::gdbstub::GdbStub;
use crate::preflight;

/// Base addresses of QEMU's real `stm32f2xx_timer` model instances on
/// `netduinoplus2` (confirmed via `info mtree`: TIM2 at 0x4000_0000, then
/// 0x400 apart). The only peripherals this backend can read alongside ITM.
const TIM2_BASE: u32 = 0x4000_0000;
/// Offset of the counter register (CNT) within a timer's register block.
const TIM_CNT_OFFSET: u32 = 0x24;

/// Whether `tool` can be spawned (i.e. exists on `PATH`). Same idiom as
/// `nucleus-cli/src/firmware.rs` and `hardware::tool_available` — duplicated
/// per-crate rather than shared, see that module's doc comment for why.
fn tool_available(tool: &str) -> bool {
    use std::io::ErrorKind;
    match Command::new(tool).arg("--version").output() {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

/// Disambiguates `itm_log_path` across multiple `QemuBackend`s started within
/// the same OS process (`std::process::id()` alone is identical for all of
/// them) — otherwise two backends in the same test binary stomp on each
/// other's semihosting log file.
static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub struct QemuBackend {
    process: Option<Child>,
    stub: Option<GdbStub>,
    skip_reason: Option<String>,
    /// Set on any real observation error (gdbstub I/O/protocol failures);
    /// deliberately NOT set for `HilError::NotObservable`, since that's an
    /// expected, documented gap (e.g. QEMU's GPIO stub), not a run failure.
    /// `finish()` reports `RunStatus::Failed` instead of `Completed` when
    /// this is set, so a flaky read mid-run can't silently look like success.
    failure: Option<String>,
    start_time: Option<Instant>,
    log: Vec<String>,
    traces: Vec<ItmEvent>,
    /// Decoded-but-not-yet-returned-to-the-caller events, in arrival order.
    /// `await_itm_event` drains this before reading more bytes.
    pending: VecDeque<ItmEvent>,
    runtime: Option<tokio::runtime::Runtime>,
    itm_log_path: Option<std::path::PathBuf>,
    /// Bytes of `itm_log_path` already decoded — `await_itm_event` reads
    /// only past this on each poll. See `itm::read_new_events`'s doc comment
    /// for why re-reading from byte 0 every poll (the old behavior) is wrong.
    itm_read_offset: u64,
    itm_decoder: Decoder,
    gdb_port: u16,
}

impl QemuBackend {
    /// Records `err` as the run's failure unless it's an expected
    /// `NotObservable` gap, then returns it unchanged for `?` to propagate.
    fn record_failure(&mut self, err: HilError) -> HilError {
        if !matches!(err, HilError::NotObservable { .. }) {
            self.failure = Some(err.to_string());
        }
        err
    }
}

impl Backend for QemuBackend {
    fn name(&self) -> BackendKind {
        BackendKind::Qemu
    }

    fn start(
        &mut self,
        firmware: &FirmwareArtifact,
        check_report: &CheckReport,
    ) -> Result<(), HilError> {
        preflight::gate(check_report)?;

        if !tool_available("qemu-system-arm") {
            self.skip_reason = Some("qemu-system-arm not found on PATH".to_string());
            return Ok(());
        }

        let instance = INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let itm_log_path = std::env::temp_dir().join(format!(
            "nucleus-hil-qemu-itm-{}-{instance}.bin",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&itm_log_path);

        let gdb_port = crate::free_port().map_err(HilError::Io)?;
        self.gdb_port = gdb_port;

        let child = Command::new("qemu-system-arm")
            .arg("-machine")
            .arg("netduinoplus2")
            .arg("-cpu")
            .arg("cortex-m4")
            .arg("-kernel")
            .arg(&firmware.elf)
            .arg("-nographic")
            .arg("-gdb")
            .arg(format!("tcp::{gdb_port}"))
            .arg("-S")
            .arg("-semihosting-config")
            .arg("enable=on,target=native,chardev=hilsemi")
            .arg("-chardev")
            .arg(format!("file,id=hilsemi,path={}", itm_log_path.display()))
            .spawn()
            .map_err(HilError::Io)?;
        self.process = Some(child);
        self.itm_log_path = Some(itm_log_path);
        self.start_time = Some(Instant::now());

        let addr = format!("127.0.0.1:{gdb_port}");
        let child_ref = self.process.as_mut().expect("just spawned");
        let stub = self
            .runtime
            .get_or_insert_with(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
            .block_on(async { connect_with_retry(&addr, child_ref).await })?;
        self.stub = Some(stub);
        let runtime = self
            .runtime
            .get_or_insert_with(|| tokio::runtime::Runtime::new().expect("tokio runtime"));
        let stub = self.stub.as_mut().unwrap();
        runtime.block_on(async { stub.continue_execution().await })?;

        Ok(())
    }

    fn pin(&mut self, _port: Port, _pin_num: u8) -> Result<bool, HilError> {
        // GPIO is an unimplemented_device stub on netduinoplus2 — see the
        // module doc comment. Always NotObservable here; the hardware
        // backend is the one that actually reads GPIO.
        Err(HilError::NotObservable {
            peripheral: "GPIO (unimplemented on QEMU's netduinoplus2 machine)".to_string(),
        })
    }

    fn register(&mut self, peripheral: &str, offset: u32) -> Result<u32, HilError> {
        let base = match peripheral {
            "TIM2" => TIM2_BASE,
            other => {
                return Err(HilError::NotObservable {
                    peripheral: other.to_string(),
                })
            }
        };
        self.read_mem32(base + offset)
    }

    fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError> {
        let stub = self
            .stub
            .as_mut()
            .ok_or_else(|| HilError::Protocol("backend not started".to_string()))?;
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        // QEMU's gdbstub only answers `m` queries while halted (see
        // GdbStub::interrupt's doc comment) — pause briefly, read, resume,
        // so the firmware keeps running between observations.
        let result = runtime.block_on(async {
            stub.interrupt().await?;
            let r = stub.read_memory(addr, 4).await;
            stub.continue_execution().await?;
            r
        });
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(err) => return Err(self.record_failure(err)),
        };
        bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| HilError::Protocol("expected 4-byte read".to_string()))
    }

    fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError> {
        let stub = self
            .stub
            .as_mut()
            .ok_or_else(|| HilError::Protocol("backend not started".to_string()))?;
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        let result = runtime.block_on(async {
            stub.interrupt().await?;
            let r = stub.write_memory(addr, &value.to_le_bytes()).await;
            stub.continue_execution().await?;
            r
        });
        result.map_err(|err| self.record_failure(err))
    }

    /// Drains `pending` (events already decoded but not yet returned) before
    /// reading more bytes. A single poll can decode more than one packet —
    /// previously only the first was ever returned and the rest silently
    /// lost, since re-polling re-read the file from byte 0 and got back the
    /// same first packet forever. `itm::read_new_events` tracks a byte
    /// cursor so each poll only sees genuinely new bytes.
    fn await_itm_event(&mut self, timeout: Duration) -> Result<Option<ItmEvent>, HilError> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        let Some(path) = self.itm_log_path.clone() else {
            return Ok(None);
        };
        let runtime = self
            .runtime
            .get_or_insert_with(|| tokio::runtime::Runtime::new().expect("tokio runtime"));
        let decoder = &mut self.itm_decoder;
        let offset = &mut self.itm_read_offset;
        let result = runtime.block_on(async {
            let deadline = Instant::now() + timeout;
            loop {
                let events = itm::read_new_events(&path, decoder, offset).await?;
                if !events.is_empty() || Instant::now() >= deadline {
                    return Ok(events);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let events: Vec<ItmEvent> = match result {
            Ok(events) => events,
            Err(err) => return Err(self.record_failure(err)),
        };
        self.traces.extend(events.iter().cloned());
        self.pending.extend(events);
        Ok(self.pending.pop_front())
    }

    /// Polls TIM2's counter at a fixed interval for `duration`. GPIO isn't
    /// observable here (see module doc), so `Sample::target` reports
    /// `RegisterChanged` rather than `Pin` — each `bool` is "did TIM2's
    /// counter change since the previous poll," not a pin level. Callers
    /// (M6/M10) must match on `target` rather than assume both backends'
    /// `Sample`s mean the same thing. The trait also has no peripheral
    /// parameter for `sample` (matches issue #20's literal
    /// `sample(duration_ms)` signature), so TIM2 is the only thing this
    /// backend samples; generalizing is a M6+ concern.
    fn sample(&mut self, duration: Duration) -> Result<Sample, HilError> {
        const POLL_INTERVAL: Duration = Duration::from_millis(5);
        let start = Instant::now();
        let mut readings = Vec::new();
        let mut last = self.register("TIM2", TIM_CNT_OFFSET)?;
        while start.elapsed() < duration {
            let current = self.register("TIM2", TIM_CNT_OFFSET)?;
            readings.push((start.elapsed(), current != last));
            last = current;
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(Sample {
            target: SampleTarget::RegisterChanged {
                peripheral: "TIM2",
                offset: TIM_CNT_OFFSET,
            },
            readings,
        })
    }

    fn finish(&mut self) -> RunResult {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(path) = self.itm_log_path.take() {
            let _ = std::fs::remove_file(path);
        }
        let status = match (&self.skip_reason, &self.failure) {
            (Some(reason), _) => RunStatus::Skipped {
                reason: reason.clone(),
            },
            (None, Some(error)) => RunStatus::Failed {
                error: error.clone(),
            },
            (None, None) if self.start_time.is_some() => RunStatus::Completed,
            (None, None) => RunStatus::Skipped {
                reason: "start() was never called".to_string(),
            },
        };
        let total = self
            .start_time
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);
        RunResult {
            backend: BackendKind::Qemu,
            status,
            log: std::mem::take(&mut self.log),
            traces: std::mem::take(&mut self.traces),
            timing: RunTiming { total },
        }
    }
}

/// QEMU needs a moment to open its `-gdb` listening socket after spawn;
/// retry the connect briefly rather than failing on the first attempt. Bails
/// immediately (rather than waiting out the full timeout) if `child` has
/// already exited — e.g. a bad `-kernel` path makes QEMU exit right away.
async fn connect_with_retry(addr: &str, child: &mut Child) -> Result<GdbStub, HilError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(HilError::Protocol(format!(
                "qemu-system-arm exited before opening its gdb port (status: {status})"
            )));
        }
        match GdbStub::connect(addr).await {
            Ok(stub) => return Ok(stub),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_compiler::check;

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

    /// Reproduces the bug directly at the `QemuBackend` level (not just
    /// `itm::read_new_events` in isolation): polling after the file already
    /// has two packets must return 'O' then 'K' once each, not re-decode 'O'
    /// forever.
    #[test]
    fn await_itm_event_returns_each_packet_once_in_order() {
        let path = std::env::temp_dir().join(format!(
            "nucleus-hil-qemu-test-{}-{}.bin",
            std::process::id(),
            INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, [0x01, b'O', 0x01, b'K']).unwrap();

        let mut backend = QemuBackend {
            itm_log_path: Some(path.clone()),
            runtime: Some(tokio::runtime::Runtime::new().unwrap()),
            ..Default::default()
        };

        let first = backend
            .await_itm_event(Duration::from_millis(200))
            .unwrap()
            .expect("first packet");
        assert_eq!(first.data, b"O".to_vec());

        let second = backend
            .await_itm_event(Duration::from_millis(200))
            .unwrap()
            .expect("second packet must not have been dropped");
        assert_eq!(second.data, b"K".to_vec());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn finish_reports_failed_not_completed_after_a_recorded_failure() {
        let mut backend = QemuBackend {
            start_time: Some(Instant::now()),
            ..Default::default()
        };
        backend.record_failure(HilError::Protocol("target hung up mid-run".to_string()));
        let result = backend.finish();
        assert!(matches!(result.status, RunStatus::Failed { .. }));
    }

    #[test]
    fn record_failure_ignores_expected_not_observable_gaps() {
        let mut backend = QemuBackend {
            start_time: Some(Instant::now()),
            ..Default::default()
        };
        backend.record_failure(HilError::NotObservable {
            peripheral: "GPIO".to_string(),
        });
        let result = backend.finish();
        assert_eq!(result.status, RunStatus::Completed);
    }

    #[test]
    fn rejects_conflicting_config_before_spawning_qemu() {
        let mut backend = QemuBackend::default();
        let firmware = FirmwareArtifact {
            elf: "unused.elf".into(),
            bin: "unused.bin".into(),
        };
        let result = backend.start(&firmware, &conflicting_report());
        assert!(matches!(result, Err(HilError::Preflight(_))));
        assert!(backend.process.is_none());
    }

    /// Boots QEMU with no `-kernel` (reset vector all-zero memory) and proves
    /// the GDB-RSP round trip works end to end against a real
    /// `qemu-system-arm` process — skipped, not failed, if QEMU isn't
    /// installed. The firmware fixture (TDD step 8) replaces the missing
    /// `-kernel` once it exists; this test only needs the boot+gdbstub
    /// plumbing to be real.
    #[test]
    fn boots_qemu_and_reads_memory_over_gdbstub() {
        if !tool_available("qemu-system-arm") {
            eprintln!("skipping: qemu-system-arm not installed");
            return;
        }

        let port = crate::free_port().expect("free port");
        let mut child = Command::new("qemu-system-arm")
            .arg("-machine")
            .arg("netduinoplus2")
            .arg("-cpu")
            .arg("cortex-m4")
            .arg("-nographic")
            .arg("-gdb")
            .arg(format!("tcp::{port}"))
            .arg("-S")
            .spawn()
            .expect("spawn qemu");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result: Result<Vec<u8>, HilError> = runtime.block_on(async {
            let mut stub = connect_with_retry(&format!("127.0.0.1:{port}"), &mut child).await?;
            stub.read_memory(0, 4).await
        });

        let _ = child.kill();
        let _ = child.wait();

        let bytes = result.expect("memory read over gdbstub");
        assert_eq!(bytes.len(), 4);
    }
}
