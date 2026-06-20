//! Executes one [`CompiledTest`]'s [`Assertion`] against a live [`Backend`].
//!
//! Every function here only *observes* — `start()`/`finish()` are the
//! caller's job (see [`crate::run_tests`]). Panic-free throughout: a malformed
//! pin string, a backend error, or a timeout all become a [`TestOutcome`],
//! never a panic.

use std::str::FromStr;
use std::time::{Duration, Instant};

use nucleus_compiler::{Assertion, CompiledTest};
use nucleus_db::Pin;

use crate::backend::{Backend, HilError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    /// This backend doesn't apply to this test (`CompiledTest::backend`
    /// selected the other one), or the backend itself couldn't be observed
    /// for a non-fatal reason (e.g. `HilError::NotObservable`). Not a
    /// failure — same "Skipped is not Failed" precedent as
    /// `RunStatus::Skipped` in `backend.rs`.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    pub name: String,
    pub status: TestStatus,
    pub detail: String,
}

/// Maps a [`HilError`] to the right [`TestStatus`]: `NotObservable` is a
/// `Skipped` (the backend simply can't see this peripheral), everything else
/// is a `Failed`.
fn outcome_from_error(name: &str, err: HilError) -> TestOutcome {
    match err {
        HilError::NotObservable { peripheral } => TestOutcome {
            name: name.to_string(),
            status: TestStatus::Skipped,
            detail: format!("peripheral not observable on this backend: {peripheral}"),
        },
        other => TestOutcome {
            name: name.to_string(),
            status: TestStatus::Failed,
            detail: other.to_string(),
        },
    }
}

/// Run one [`CompiledTest`]'s assertion against `backend`, which must already
/// be started ([`Backend::start`] called and `Ok`) — this function only
/// observes, never boots/flashes.
pub fn run(backend: &mut dyn Backend, test: &CompiledTest) -> TestOutcome {
    match &test.assertion {
        Assertion::PinToggles {
            pin,
            hz,
            tolerance_pct,
        } => run_pin_toggles(backend, &test.name, pin, *hz, *tolerance_pct),
        Assertion::PinState { pin, level, within } => {
            run_pin_state(backend, &test.name, pin, *level, *within)
        }
        Assertion::UartEcho {
            instance,
            payload,
            within,
        } => run_uart_echo(backend, &test.name, instance, payload, *within),
        Assertion::ItmEvent { pattern, within } => {
            run_itm_event(backend, &test.name, pattern, *within)
        }
    }
}

fn parse_pin_or_fail(name: &str, pin: &str) -> Result<Pin, TestOutcome> {
    Pin::from_str(pin).map_err(|_| TestOutcome {
        name: name.to_string(),
        status: TestStatus::Failed,
        detail: format!("internal error: pin {pin:?} failed to parse despite compiler validation"),
    })
}

fn run_pin_toggles(
    backend: &mut dyn Backend,
    name: &str,
    pin: &str,
    hz: f64,
    tolerance_pct: f64,
) -> TestOutcome {
    if let Err(outcome) = parse_pin_or_fail(name, pin) {
        return outcome;
    }

    // At least 3 full periods, flooring `hz` to avoid a division blowup on a
    // malformed-but-parsed near-zero frequency.
    let window = Duration::from_secs_f64(3.0 / hz.max(0.1));
    let sample = match backend.sample(window) {
        Ok(sample) => sample,
        Err(err) => return outcome_from_error(name, err),
    };

    let rising_edges = sample
        .readings
        .windows(2)
        .filter(|pair| !pair[0].1 && pair[1].1)
        .count();

    let window_secs = window.as_secs_f64();
    let measured_hz = if window_secs > 0.0 {
        rising_edges as f64 / window_secs
    } else {
        0.0
    };

    let tolerance = hz.abs() * (tolerance_pct / 100.0);
    if (measured_hz - hz).abs() <= tolerance {
        TestOutcome {
            name: name.to_string(),
            status: TestStatus::Passed,
            detail: format!(
                "measured {measured_hz:.2} Hz, expected {hz:.2} Hz (within {tolerance_pct}%)"
            ),
        }
    } else {
        TestOutcome {
            name: name.to_string(),
            status: TestStatus::Failed,
            detail: format!("measured {measured_hz:.2} Hz, expected {hz:.2} Hz ± {tolerance_pct}%"),
        }
    }
}

fn run_pin_state(
    backend: &mut dyn Backend,
    name: &str,
    pin: &str,
    level: bool,
    within: Duration,
) -> TestOutcome {
    let parsed = match parse_pin_or_fail(name, pin) {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };

    let start = Instant::now();
    loop {
        match backend.pin(parsed.port, parsed.number) {
            Ok(reading) if reading == level => {
                return TestOutcome {
                    name: name.to_string(),
                    status: TestStatus::Passed,
                    detail: format!("{pin} reached level {level}"),
                };
            }
            Ok(_) => {
                if start.elapsed() > within {
                    return TestOutcome {
                        name: name.to_string(),
                        status: TestStatus::Failed,
                        detail: format!("{pin} never reached level {level} within {within:?}"),
                    };
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(err) => return outcome_from_error(name, err),
        }
    }
}

fn run_uart_echo(
    backend: &mut dyn Backend,
    name: &str,
    instance: &str,
    payload: &[u8],
    within: Duration,
) -> TestOutcome {
    match backend.await_itm_event(within) {
        Ok(Some(event)) => {
            let found =
                payload.is_empty() || event.data.windows(payload.len()).any(|w| w == payload);
            if found {
                TestOutcome {
                    name: name.to_string(),
                    status: TestStatus::Passed,
                    detail: format!("observed echoed payload on {instance}"),
                }
            } else {
                TestOutcome {
                    name: name.to_string(),
                    status: TestStatus::Failed,
                    detail: format!(
                        "ITM event observed but did not contain expected payload for {instance}"
                    ),
                }
            }
        }
        Ok(None) => TestOutcome {
            name: name.to_string(),
            status: TestStatus::Failed,
            detail: format!("no ITM event observed for {instance} within {within:?}"),
        },
        Err(err) => outcome_from_error(name, err),
    }
}

fn run_itm_event(
    backend: &mut dyn Backend,
    name: &str,
    pattern: &str,
    within: Duration,
) -> TestOutcome {
    match backend.await_itm_event(within) {
        Ok(Some(event)) => {
            let pattern_bytes = pattern.as_bytes();
            let found = pattern_bytes.is_empty()
                || event
                    .data
                    .windows(pattern_bytes.len())
                    .any(|w| w == pattern_bytes);
            if found {
                TestOutcome {
                    name: name.to_string(),
                    status: TestStatus::Passed,
                    detail: format!("ITM event matched pattern {pattern:?}"),
                }
            } else {
                TestOutcome {
                    name: name.to_string(),
                    status: TestStatus::Failed,
                    detail: format!("ITM event observed but did not match pattern {pattern:?}"),
                }
            }
        }
        Ok(None) => TestOutcome {
            name: name.to_string(),
            status: TestStatus::Failed,
            detail: format!("no ITM event matching {pattern:?} observed within {within:?}"),
        },
        Err(err) => outcome_from_error(name, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        BackendKind, FirmwareArtifact, ItmEvent, RunResult, RunStatus, RunTiming, Sample,
        SampleTarget,
    };
    use nucleus_compiler::{BackendSelect, CheckReport};
    use std::cell::Cell;

    /// A controllable fake `Backend` for driving each `Assertion` arm
    /// deterministically — no real timing/sleeps involved beyond `PinState`'s
    /// own short poll loop, which we bound with a tiny `within`.
    struct FakeBackend {
        pin_result: Result<bool, HilErrorKind>,
        sample_result: Result<Sample, HilErrorKind>,
        itm_result: Result<Option<ItmEvent>, HilErrorKind>,
        pin_calls: Cell<u32>,
        sample_calls: Cell<u32>,
        itm_calls: Cell<u32>,
    }

    /// A cloneable stand-in for `HilError` (which isn't `Clone`) so the fake
    /// can be constructed once and reused across calls within a test.
    #[derive(Clone)]
    enum HilErrorKind {
        NotObservable(String),
        Protocol(String),
    }

    impl HilErrorKind {
        fn into_hil_error(self) -> HilError {
            match self {
                HilErrorKind::NotObservable(peripheral) => HilError::NotObservable { peripheral },
                HilErrorKind::Protocol(msg) => HilError::Protocol(msg),
            }
        }
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            FakeBackend {
                pin_result: Ok(false),
                sample_result: Ok(Sample {
                    target: SampleTarget::Pin {
                        port: nucleus_db::Port::A,
                        pin_num: 5,
                    },
                    readings: vec![],
                }),
                itm_result: Ok(None),
                pin_calls: Cell::new(0),
                sample_calls: Cell::new(0),
                itm_calls: Cell::new(0),
            }
        }
    }

    impl Backend for FakeBackend {
        fn name(&self) -> BackendKind {
            BackendKind::Hardware
        }

        fn start(
            &mut self,
            _firmware: &FirmwareArtifact,
            _check_report: &CheckReport,
        ) -> Result<(), HilError> {
            Ok(())
        }

        fn pin(&mut self, _port: nucleus_db::Port, _pin_num: u8) -> Result<bool, HilError> {
            self.pin_calls.set(self.pin_calls.get() + 1);
            self.pin_result
                .clone()
                .map_err(HilErrorKind::into_hil_error)
        }

        fn register(&mut self, _peripheral: &str, _offset: u32) -> Result<u32, HilError> {
            Ok(0)
        }

        fn await_itm_event(&mut self, _timeout: Duration) -> Result<Option<ItmEvent>, HilError> {
            self.itm_calls.set(self.itm_calls.get() + 1);
            self.itm_result
                .clone()
                .map_err(HilErrorKind::into_hil_error)
        }

        fn sample(&mut self, _duration: Duration) -> Result<Sample, HilError> {
            self.sample_calls.set(self.sample_calls.get() + 1);
            self.sample_result
                .clone()
                .map_err(HilErrorKind::into_hil_error)
        }

        fn finish(&mut self) -> RunResult {
            RunResult {
                backend: self.name(),
                status: RunStatus::Completed,
                log: vec![],
                traces: vec![],
                timing: RunTiming::default(),
            }
        }
    }

    fn toggles_readings(hz: f64, window: Duration) -> Vec<(Duration, bool)> {
        // Build a clean square wave at `hz` over `window`, starting from a
        // `false` baseline reading, so the rising-edge count divides back out
        // to exactly `hz` (one rising edge per full period).
        let period = Duration::from_secs_f64(1.0 / hz);
        let mut readings = vec![(Duration::ZERO, false)];
        let mut t = period / 2;
        let mut level = false;
        while t <= window {
            level = !level;
            readings.push((t, level));
            t += period / 2;
        }
        readings
    }

    fn test_with(assertion: Assertion, backend_select: BackendSelect) -> CompiledTest {
        CompiledTest {
            name: "t".to_string(),
            assertion,
            timeout: Duration::from_millis(50),
            backend: backend_select,
        }
    }

    #[test]
    fn pin_toggles_within_tolerance_passes() {
        let hz = 100.0;
        let window = Duration::from_secs_f64(3.0 / hz);
        let mut backend = FakeBackend {
            sample_result: Ok(Sample {
                target: SampleTarget::Pin {
                    port: nucleus_db::Port::A,
                    pin_num: 5,
                },
                readings: toggles_readings(hz, window),
            }),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::PinToggles {
                pin: "PA5".to_string(),
                hz,
                tolerance_pct: 10.0,
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Passed, "{}", outcome.detail);
    }

    #[test]
    fn pin_toggles_outside_tolerance_fails() {
        let hz = 100.0;
        let window = Duration::from_secs_f64(3.0 / hz);
        // Half the expected edges -> way outside tolerance.
        let mut backend = FakeBackend {
            sample_result: Ok(Sample {
                target: SampleTarget::Pin {
                    port: nucleus_db::Port::A,
                    pin_num: 5,
                },
                readings: toggles_readings(hz / 4.0, window),
            }),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::PinToggles {
                pin: "PA5".to_string(),
                hz,
                tolerance_pct: 5.0,
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Failed);
    }

    #[test]
    fn pin_state_matching_level_passes() {
        let mut backend = FakeBackend {
            pin_result: Ok(true),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::PinState {
                pin: "PA5".to_string(),
                level: true,
                within: Duration::from_millis(20),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Passed);
        assert_eq!(backend.pin_calls.get(), 1);
    }

    #[test]
    fn pin_state_never_matching_fails_after_timeout() {
        let mut backend = FakeBackend {
            pin_result: Ok(false),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::PinState {
                pin: "PA5".to_string(),
                level: true,
                within: Duration::from_millis(5),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Failed);
        assert!(backend.pin_calls.get() > 0);
    }

    #[test]
    fn uart_echo_with_matching_payload_passes() {
        let mut backend = FakeBackend {
            itm_result: Ok(Some(ItmEvent {
                port: 0,
                data: b"prefix:HELLO:suffix".to_vec(),
            })),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::UartEcho {
                instance: "usart2".to_string(),
                payload: b"HELLO".to_vec(),
                within: Duration::from_millis(20),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Passed);
    }

    #[test]
    fn uart_echo_with_no_event_fails() {
        let mut backend = FakeBackend {
            itm_result: Ok(None),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::UartEcho {
                instance: "usart2".to_string(),
                payload: b"HELLO".to_vec(),
                within: Duration::from_millis(20),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Failed);
    }

    #[test]
    fn itm_event_matching_pattern_passes() {
        let mut backend = FakeBackend {
            itm_result: Ok(Some(ItmEvent {
                port: 0,
                data: b"boot complete".to_vec(),
            })),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::ItmEvent {
                pattern: "complete".to_string(),
                within: Duration::from_millis(20),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Passed);
    }

    #[test]
    fn itm_event_non_matching_pattern_fails() {
        let mut backend = FakeBackend {
            itm_result: Ok(Some(ItmEvent {
                port: 0,
                data: b"boot failed".to_vec(),
            })),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::ItmEvent {
                pattern: "complete".to_string(),
                within: Duration::from_millis(20),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Failed);
    }

    #[test]
    fn itm_event_timeout_with_no_event_fails() {
        let mut backend = FakeBackend {
            itm_result: Ok(None),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::ItmEvent {
                pattern: "complete".to_string(),
                within: Duration::from_millis(20),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Failed);
    }

    #[test]
    fn not_observable_error_is_skipped_not_failed() {
        let mut backend = FakeBackend {
            pin_result: Err(HilErrorKind::NotObservable("GPIOA".to_string())),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::PinState {
                pin: "PA5".to_string(),
                level: true,
                within: Duration::from_millis(5),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Skipped);
    }

    #[test]
    fn protocol_error_is_failed_not_skipped() {
        let mut backend = FakeBackend {
            sample_result: Err(HilErrorKind::Protocol("truncated reply".to_string())),
            ..FakeBackend::default()
        };
        let test = test_with(
            Assertion::PinToggles {
                pin: "PA5".to_string(),
                hz: 10.0,
                tolerance_pct: 5.0,
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &test);
        assert_eq!(outcome.status, TestStatus::Failed);
    }

    #[test]
    fn malformed_pin_never_panics_and_fails_cleanly() {
        let mut backend = FakeBackend::default();
        let toggles = test_with(
            Assertion::PinToggles {
                pin: "not-a-pin".to_string(),
                hz: 10.0,
                tolerance_pct: 5.0,
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &toggles);
        assert_eq!(outcome.status, TestStatus::Failed);
        assert!(outcome.detail.contains("internal error"));

        let state = test_with(
            Assertion::PinState {
                pin: "PZZ99".to_string(),
                level: true,
                within: Duration::from_millis(5),
            },
            BackendSelect::Both,
        );
        let outcome = run(&mut backend, &state);
        assert_eq!(outcome.status, TestStatus::Failed);
        assert!(outcome.detail.contains("internal error"));
    }
}
