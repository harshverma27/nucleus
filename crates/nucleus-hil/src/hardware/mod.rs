//! The hardware HIL backend: flash a real board (`st-flash`), spawn OpenOCD
//! to bridge the ST-Link's SWD/SWO lines to TCP, observe via OpenOCD's
//! gdbserver (memory reads via [`crate::gdbstub`]) and its SWO trace port
//! (decoded by [`itm`]).
//!
//! Getting this far required two fixture-firmware fixes found only by
//! testing against real silicon (QEMU models neither gap): the SWO pin
//! (PB3 AF0) and `DBGMCU_CR.TRACE_IOEN` are off by default after reset, and
//! a `uint32_t`-wide store to the ITM stimulus port packetizes as 4 bytes on
//! real hardware even for a 1-byte payload — see
//! `tests/fixtures/blink_itm/captured_swo.README.md`.

pub mod itm;

use std::collections::VecDeque;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use nucleus_compiler::CheckReport;
use nucleus_db::Port;
use nucleus_itm::{Decoder, Packet};
use nucleus_trace::source::openocd_enable;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::backend::{
    Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, RunTiming,
};
use crate::backend::{RunStatus, Sample, SampleTarget};
use crate::gdbstub::GdbStub;
use crate::gpio_map;
use crate::preflight;

/// TIM2's APB1 base address — identical to `qemu::TIM2_BASE`; both mirror the
/// real F4 memory map here.
const TIM2_BASE: u32 = 0x4000_0000;
/// The pin `blink_itm_hw` toggles — the one signal this backend can prove
/// changing state on real GPIO that QEMU structurally cannot (see
/// `qemu::mod`'s doc comment).
const SAMPLE_PIN: (Port, u8) = (Port::A, 5);

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

/// Wait for `addr` to accept a TCP connection, bailing immediately (rather
/// than waiting out the full timeout) if `child` has already exited.
async fn wait_for_port(addr: &str, child: &mut Child) -> Result<(), HilError> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(HilError::Protocol(format!(
                "openocd exited before opening {addr} (status: {status})"
            )));
        }
        match TcpStream::connect(addr).await {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => return Err(HilError::Io(err)),
        }
    }
}

/// Connect to `addr`, retrying on `ConnectionRefused` for up to 2 seconds.
///
/// OpenOCD's `tpiu config internal :PORT ...` telnet command only binds the
/// trace TCP listener *after* it finishes processing the command — the
/// `write_all`/`flush` in [`openocd_enable`] returns as soon as the bytes hit
/// the socket, not once OpenOCD has acted on them, so connecting immediately
/// afterward loses this race and gets `ConnectionRefused` (confirmed
/// empirically against a real NUCLEO-F411RE; never observed against QEMU,
/// which has no analogous trace-port setup step).
async fn connect_with_retry(addr: &str) -> std::io::Result<TcpStream> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(err)
                if err.kind() == std::io::ErrorKind::ConnectionRefused
                    && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Send one command to OpenOCD's telnet console. Used for `reset halt` /
/// `reset run` — [`openocd_enable`] covers the TPIU/ITM setup commands.
async fn telnet_send(addr: &str, cmd: &str) -> Result<(), HilError> {
    let mut conn = TcpStream::connect(addr).await?;
    conn.write_all(cmd.as_bytes()).await?;
    conn.write_all(b"\r\n").await?;
    conn.flush().await?;
    // A short settle delay after each blocking telnet command, before this
    // connection drops and/or the next command fires. Without it, the SWO
    // trace pipeline below silently received zero bytes end-to-end on real
    // hardware (every command appeared to succeed — no error from OpenOCD —
    // yet no SWIT packet ever arrived); adding this delay here and after
    // `openocd_enable` made it reliable. Root cause not fully isolated
    // (reset settling on real silicon vs. a connection-drop race in
    // OpenOCD's command dispatcher are both plausible); confirmed empirically
    // against a real NUCLEO-F411RE, below the noise floor on QEMU.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

#[derive(Default)]
pub struct HardwareBackend {
    started: bool,
    skip_reason: Option<String>,
    /// Set on any real observation error (gdbstub/SWO I/O or protocol
    /// failures) after a successful `start()`; deliberately NOT set for
    /// `HilError::NotObservable`, since that's an expected, documented gap,
    /// not a run failure. `finish()` reports `RunStatus::Failed` instead of
    /// `Completed` when this is set.
    failure: Option<String>,
    start_time: Option<Instant>,
    log: Vec<String>,
    traces: Vec<ItmEvent>,
    /// Decoded-but-not-yet-returned-to-the-caller events, in arrival order.
    /// `await_itm_event` drains this before reading more bytes off the live
    /// socket — bytes already pulled off it can't be re-read, so every
    /// packet a single read decodes must be kept, not just the first.
    pending: VecDeque<ItmEvent>,
    runtime: Option<tokio::runtime::Runtime>,
    openocd: Option<Child>,
    stub: Option<GdbStub>,
    trace_reader: Option<Box<dyn AsyncRead + Unpin + Send>>,
    decoder: Decoder,
    telnet_port: u16,
    gdb_port: u16,
    trace_port: u16,
}

impl HardwareBackend {
    /// Records `err` as the run's failure unless it's an expected
    /// `NotObservable` gap, then returns it unchanged for `?` to propagate.
    fn record_failure(&mut self, err: HilError) -> HilError {
        if !matches!(err, HilError::NotObservable { .. }) {
            self.failure = Some(err.to_string());
        }
        err
    }

    /// Halt, read 4 bytes at `addr` over the gdbserver, resume. Mirrors
    /// `QemuBackend::register`'s halt/read/resume dance — OpenOCD's gdbserver
    /// only answers `m` queries while the target is halted.
    fn read_memory(&mut self, addr: u32) -> Result<u32, HilError> {
        if !self.started {
            return Err(HilError::Protocol("backend not started".to_string()));
        }
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        if self.stub.is_none() {
            // Connected lazily, on first actual register/pin read, rather
            // than eagerly in `start()` — opening the gdb-port connection
            // immediately after `reset run` raced OpenOCD's own SWO-capture
            // loop on real hardware (`Error: attempted 'gdb' connection
            // rejected` in its log, immediately followed by zero ITM bytes
            // ever arriving), confirmed empirically against a NUCLEO-F411RE.
            // A test that never calls `pin`/`register`/`sample` now never
            // opens this connection at all.
            let gdb_addr = format!("127.0.0.1:{}", self.gdb_port);
            let stub = match runtime.block_on(GdbStub::connect(&gdb_addr)) {
                Ok(stub) => stub,
                Err(err) => return Err(self.record_failure(err)),
            };
            self.stub = Some(stub);
        }
        let stub = self.stub.as_mut().expect("just set above");
        let result = runtime.block_on(async {
            stub.interrupt().await?;
            let result = stub.read_memory(addr, 4).await;
            stub.continue_execution().await?;
            result
        });
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(err) => return Err(self.record_failure(err)),
        };
        bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| HilError::Protocol("expected 4-byte register read".to_string()))
    }
}

impl Backend for HardwareBackend {
    fn name(&self) -> BackendKind {
        BackendKind::Hardware
    }

    fn start(
        &mut self,
        firmware: &FirmwareArtifact,
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
        if !tool_available("openocd") {
            self.skip_reason = Some("openocd not found on PATH".to_string());
            return Ok(());
        }

        match Command::new("st-flash")
            .arg("write")
            .arg(&firmware.bin)
            .arg("0x08000000")
            .output()
        {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                return Err(HilError::Protocol(format!(
                    "st-flash failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                )))
            }
            Err(err) => return Err(HilError::Io(err)),
        }

        // Dynamically allocated rather than OpenOCD's textbook defaults
        // (4444/3333/3344) so two HardwareBackend runs in the same process
        // don't collide on a port (e.g. parallel test threads) — even
        // though only one can ever touch the single physical board at a
        // time, the port-bind step itself would otherwise race.
        self.telnet_port = crate::free_port().map_err(HilError::Io)?;
        self.gdb_port = crate::free_port().map_err(HilError::Io)?;
        self.trace_port = crate::free_port().map_err(HilError::Io)?;

        let mut openocd = Command::new("openocd")
            .arg("-f")
            .arg("interface/stlink.cfg")
            .arg("-f")
            .arg("target/stm32f4x.cfg")
            .arg("-c")
            .arg(format!("gdb_port {}", self.gdb_port))
            .arg("-c")
            .arg(format!("telnet_port {}", self.telnet_port))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(HilError::Io)?;

        let telnet_addr = format!("127.0.0.1:{}", self.telnet_port);
        let trace_addr = format!("127.0.0.1:{}", self.trace_port);
        let trace_port = self.trace_port;

        // No PLL is configured on a freshly flashed board until the
        // firmware's own init runs, so the core is still on its reset clock
        // (HSI, 16 MHz on F4) — a different default than the CLI's
        // already-configured-project assumption of 180 MHz.
        let cpu_hz = check_report.config.device.clock_hz.unwrap_or(16_000_000) as u32;
        let swo_hz = check_report.config.trace.swo_freq.unwrap_or(2_000_000) as u32;

        let runtime = self
            .runtime
            .get_or_insert_with(|| tokio::runtime::Runtime::new().expect("tokio runtime"));

        let result: Result<Box<dyn AsyncRead + Unpin + Send>, HilError> = runtime.block_on(async {
            wait_for_port(&telnet_addr, &mut openocd).await?;
            // Halt before configuring TPIU/ITM and opening the trace
            // socket, then resume only once a reader is attached —
            // bytes written to a not-yet-connected SWO trace port are
            // dropped, not buffered (confirmed empirically).
            telnet_send(&telnet_addr, "reset halt").await?;
            openocd_enable(&telnet_addr, trace_port, cpu_hz, swo_hz)
                .await
                .map_err(HilError::Io)?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let trace_reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(
                connect_with_retry(&trace_addr)
                    .await
                    .map_err(HilError::Io)?,
            );
            // `itm ports on` accepting the trace TCP connection doesn't
            // imply the TPIU/DBGMCU trace-enable bits have actually
            // latched in hardware yet — resuming immediately on a real
            // board occasionally resumes before that lands, dropping the
            // firmware's very first SWIT packet (confirmed empirically
            // against a NUCLEO-F411RE; below the noise floor on QEMU's
            // SWO emulation, which has no equivalent hardware latency).
            tokio::time::sleep(Duration::from_millis(1500)).await;
            telnet_send(&telnet_addr, "reset run").await?;
            Ok(trace_reader)
        });

        match result {
            Ok(trace_reader) => {
                self.openocd = Some(openocd);
                self.trace_reader = Some(trace_reader);
                self.started = true;
                self.start_time = Some(Instant::now());
                Ok(())
            }
            Err(err) => {
                let _ = openocd.kill();
                let _ = openocd.wait();
                Err(err)
            }
        }
    }

    fn pin(&mut self, port: Port, pin_num: u8) -> Result<bool, HilError> {
        let value = self.read_memory(gpio_map::idr_address(port))?;
        Ok((value >> pin_num) & 1 != 0)
    }

    fn register(&mut self, peripheral: &str, offset: u32) -> Result<u32, HilError> {
        let base = match peripheral {
            "GPIOA" => gpio_map::gpio_base(Port::A),
            "GPIOB" => gpio_map::gpio_base(Port::B),
            "GPIOC" => gpio_map::gpio_base(Port::C),
            "GPIOD" => gpio_map::gpio_base(Port::D),
            "GPIOE" => gpio_map::gpio_base(Port::E),
            "GPIOF" => gpio_map::gpio_base(Port::F),
            "GPIOG" => gpio_map::gpio_base(Port::G),
            "GPIOH" => gpio_map::gpio_base(Port::H),
            "TIM2" => TIM2_BASE,
            other => {
                return Err(HilError::NotObservable {
                    peripheral: other.to_string(),
                })
            }
        };
        self.read_memory(base + offset)
    }

    /// Drains `pending` (events already decoded but not yet returned)
    /// before reading more bytes. A single read can decode more than one
    /// packet (real hardware's "OK" log arrives as two back-to-back SWIT
    /// packets in one TCP read) — unlike `QemuBackend`'s file-replay poll, a
    /// live socket's bytes can't be re-read, so every packet a read decodes
    /// is queued, not just the first.
    fn await_itm_event(&mut self, timeout: Duration) -> Result<Option<ItmEvent>, HilError> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        let Some(reader) = self.trace_reader.as_mut() else {
            return Ok(None);
        };
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        let decoder = &mut self.decoder;

        let result: Result<Vec<ItmEvent>, HilError> = runtime.block_on(async {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(Vec::new());
                }
                let mut buf = [0u8; 256];
                let n = match tokio::time::timeout(remaining, reader.read(&mut buf)).await {
                    Ok(Ok(n)) => n,
                    Ok(Err(err)) => return Err(HilError::Io(err)),
                    Err(_) => return Ok(Vec::new()),
                };
                if n == 0 {
                    return Ok(Vec::new());
                }
                let events: Vec<ItmEvent> = decoder
                    .decode(&buf[..n])
                    .into_iter()
                    .filter_map(|packet| match packet {
                        Packet::Instrumentation { port, data } => Some(ItmEvent { port, data }),
                        _ => None,
                    })
                    .collect();
                if !events.is_empty() {
                    return Ok(events);
                }
            }
        });

        let events = match result {
            Ok(events) => events,
            Err(err) => return Err(self.record_failure(err)),
        };
        self.traces.extend(events.iter().cloned());
        self.pending.extend(events);
        Ok(self.pending.pop_front())
    }

    /// Polls `SAMPLE_PIN` (PA5, the pin `blink_itm_hw` toggles) over
    /// `duration`. Unlike `QemuBackend::sample` (which has to fall back to
    /// TIM2 — see its doc comment), this backend reads the real pin: GPIO is
    /// not an `unimplemented_device` stub on actual silicon. `Sample::target`
    /// reports `Pin` so callers can tell the two backends' `Sample`s apart
    /// rather than assuming they mean the same thing.
    fn sample(&mut self, duration: Duration) -> Result<Sample, HilError> {
        const POLL_INTERVAL: Duration = Duration::from_millis(5);
        let (port, pin_num) = SAMPLE_PIN;
        let start = Instant::now();
        let mut readings = Vec::new();
        while start.elapsed() < duration {
            let state = self.pin(port, pin_num)?;
            readings.push((start.elapsed(), state));
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(Sample {
            target: SampleTarget::Pin { port, pin_num },
            readings,
        })
    }

    fn finish(&mut self) -> RunResult {
        if let Some(mut child) = self.openocd.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.trace_reader = None;
        self.stub = None;
        let status = match (&self.skip_reason, &self.failure) {
            (Some(reason), _) => RunStatus::Skipped {
                reason: reason.clone(),
            },
            (None, Some(error)) => RunStatus::Failed {
                error: error.clone(),
            },
            (None, None) if self.started => RunStatus::Completed,
            (None, None) => RunStatus::Skipped {
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

/// Kills the spawned `openocd` child if `finish()` was never reached — e.g. a
/// test panics between `start()` and `finish()` (an `.expect()` on a failed
/// observation call, for instance). Without this, a panicking test leaks an
/// `openocd` process that holds the ST-Link exclusively and the fixed tcl
/// port 6666, so every subsequent run (this process or the next) fails to
/// even spawn its own `openocd` — confirmed empirically: a single panicking
/// hardware test left exactly this orphan behind and broke every later
/// attempt until it was killed by hand.
impl Drop for HardwareBackend {
    fn drop(&mut self) {
        if let Some(mut child) = self.openocd.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_compiler::check;
    use std::path::PathBuf;

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

    /// No board needed: a real in-process TCP socket stands in for the live
    /// SWO stream. Reproduces the bug directly — a single read delivering
    /// both packets must not lose the second one on a later poll.
    #[test]
    fn await_itm_event_returns_each_packet_once_in_order_without_a_board() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let trace_reader: Box<dyn AsyncRead + Unpin + Send> = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (accept_result, connect_result) =
                tokio::join!(listener.accept(), TcpStream::connect(addr));
            let (mut server, _) = accept_result.unwrap();
            server.write_all(&[0x01, b'O', 0x01, b'K']).await.unwrap();
            Box::new(connect_result.unwrap())
        });

        let mut backend = HardwareBackend::default();
        backend.trace_reader = Some(trace_reader);
        backend.runtime = Some(runtime);

        let first = backend
            .await_itm_event(Duration::from_millis(500))
            .unwrap()
            .expect("first packet");
        assert_eq!(first.data, b"O".to_vec());

        let second = backend
            .await_itm_event(Duration::from_millis(500))
            .unwrap()
            .expect("second packet must not have been dropped");
        assert_eq!(second.data, b"K".to_vec());

        assert!(backend
            .await_itm_event(Duration::from_millis(50))
            .unwrap()
            .is_none());
    }

    #[test]
    fn finish_reports_failed_not_completed_after_a_recorded_failure() {
        let mut backend = HardwareBackend::default();
        backend.started = true;
        backend.start_time = Some(Instant::now());
        backend.record_failure(HilError::Protocol("board disconnected mid-run".to_string()));
        let result = backend.finish();
        assert!(matches!(result.status, RunStatus::Failed { .. }));
    }

    #[test]
    fn record_failure_ignores_expected_not_observable_gaps() {
        let mut backend = HardwareBackend::default();
        backend.started = true;
        backend.start_time = Some(Instant::now());
        backend.record_failure(HilError::NotObservable {
            peripheral: "SAI1".to_string(),
        });
        let result = backend.finish();
        assert_eq!(result.status, RunStatus::Completed);
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
        if board_detected() {
            eprintln!("skipping: this test machine has a board attached");
            return;
        }
        let mut backend = HardwareBackend::default();
        let firmware = FirmwareArtifact {
            elf: "unused.elf".into(),
            bin: "unused.bin".into(),
        };
        backend.start(&firmware, &clean_report()).unwrap();
        let result = backend.finish();
        assert!(matches!(result.status, RunStatus::Skipped { .. }));
    }

    /// The real exit criterion this module exists for: flash `blink_itm_hw`
    /// to whatever board is attached, observe its ITM log and its PA5 toggle
    /// over real SWD/SWO — skipped, not failed, if no board/tooling is
    /// present (CI has neither).
    #[test]
    fn flashes_and_observes_a_real_board_when_present() {
        if !tool_available("st-flash") || !tool_available("openocd") || !board_detected() {
            eprintln!("skipping: st-flash/openocd/board not all available");
            return;
        }

        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/blink_itm");
        let firmware = FirmwareArtifact {
            elf: fixtures.join("blink_itm_hw.elf"),
            bin: fixtures.join("blink_itm_hw.bin"),
        };

        let mut backend = HardwareBackend::default();
        backend
            .start(&firmware, &clean_report())
            .expect("start succeeds with board present");

        // blink_itm_hw emits two single-byte SWIT packets ('O', 'K') back to
        // back. Each must come back exactly once, in order — the bug this
        // session fixed silently dropped the second one.
        let first = backend
            .await_itm_event(Duration::from_secs(3))
            .expect("itm read doesn't error")
            .expect("blink_itm_hw emits an ITM log at boot");
        assert_eq!(first.port, 0);
        assert_eq!(first.data, b"O".to_vec());

        let second = backend
            .await_itm_event(Duration::from_millis(200))
            .expect("itm read doesn't error")
            .expect("second packet ('K') must not be dropped");
        assert_eq!(second.data, b"K".to_vec());

        let sample = backend
            .sample(Duration::from_millis(500))
            .expect("pin read doesn't error");
        assert_eq!(
            sample.target,
            SampleTarget::Pin {
                port: Port::A,
                pin_num: 5
            }
        );
        assert!(
            sample.readings.iter().any(|(_, state)| *state),
            "PA5 should be high at least once while blink_itm_hw toggles it"
        );

        let result = backend.finish();
        assert_eq!(result.status, RunStatus::Completed);
    }
}
