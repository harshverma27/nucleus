//! M10 — lockstep co-execution. Run the same firmware on two [`Backend`]s,
//! collect one [`ObservationTrace`] per backend from the ITM events each
//! emits, and [`compare`] them to find the first point they disagree.
//!
//! Sync points are ITM events (both backends already expose
//! [`Backend::await_itm_event`]); observables are whatever
//! `[trace.variables]` decodes a port's bytes into. No new backend calls are
//! needed per checkpoint — a traced variable's value already arrives inside
//! the ITM packet that names its port, so [`collect`] just decodes what
//! [`Backend::await_itm_event`] already captured via the same
//! [`nucleus_trace::translate::Translator`] the trace daemon uses.
//!
//! No explanation engine here (deferred past v2) — [`DivergenceReport`] is a
//! bare diff: which checkpoint, which observable, what each side saw.

use std::time::{Duration, Instant};

use nucleus_itm::Packet;
use nucleus_trace::translate::{Translator, VariableMap};
use serde_json::Value;

use crate::backend::{Backend, ItmEvent};

/// One sync point: the raw ITM event observed, plus its decoded
/// `(name, value)` if its port matches a `[trace.variables]` entry. Port-0
/// log lines and unconfigured ports carry `decoded: None` — they still
/// compare on the raw event.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub itm_event: ItmEvent,
    pub decoded: Option<(String, Value)>,
}

/// Every checkpoint collected from one backend's run, in observation order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObservationTrace {
    pub checkpoints: Vec<Checkpoint>,
}

/// The bare-diff result of comparing two [`ObservationTrace`]s. No
/// inference, no explanation — just where and what.
#[derive(Debug, Clone, PartialEq)]
pub enum DivergenceReport {
    /// Both traces had the same length and agreed at every checkpoint.
    Agreement { checkpoints_compared: usize },
    /// The first index where the two traces disagree.
    Diverged {
        first_checkpoint: usize,
        /// The variable name that differed, or `"itm_event"` if the raw
        /// event itself differed (or one side ran out of checkpoints).
        observable: String,
        sim_value: String,
        silicon_value: String,
    },
}

/// Drive `backend` with [`Backend::await_itm_event`] until it returns
/// `Ok(None)` (no more events within `timeout_per_event`) or `total_timeout`
/// elapses, decoding each event against `vars`. `backend` must already be
/// started ([`Backend::start`] called and `Ok`) — this only observes.
///
/// A backend error mid-collection stops the loop and returns whatever
/// checkpoints were gathered so far; callers compare partial traces the
/// same as complete ones (a length mismatch is itself a divergence).
pub fn collect(
    backend: &mut dyn Backend,
    vars: &VariableMap,
    timeout_per_event: Duration,
    total_timeout: Duration,
) -> ObservationTrace {
    let mut checkpoints = Vec::new();
    let deadline = Instant::now() + total_timeout;

    loop {
        if Instant::now() >= deadline {
            break;
        }
        match backend.await_itm_event(timeout_per_event) {
            Ok(Some(event)) => {
                let decoded = decode(vars, &event);
                checkpoints.push(Checkpoint {
                    itm_event: event,
                    decoded,
                });
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    ObservationTrace { checkpoints }
}

/// Decode `event` against `vars` by replaying it through the same
/// [`Translator`] the trace daemon uses, returning the first decoded
/// variable (a configured port produces exactly one). `None` for port-0 log
/// lines and ports with no `[trace.variables]` entry.
fn decode(vars: &VariableMap, event: &ItmEvent) -> Option<(String, Value)> {
    let mut translator = Translator::new(vars.clone());
    let packet = Packet::Instrumentation {
        port: event.port,
        data: event.data.clone(),
    };
    translator.translate(&packet).into_iter().find_map(|te| {
        if let nucleus_trace::translate::TraceEvent::Variable { name, value, .. } = te {
            Some((name, value))
        } else {
            None
        }
    })
}

/// Compare two traces checkpoint by checkpoint, stopping at the first
/// disagreement. Equal length and equal content at every index is
/// [`DivergenceReport::Agreement`]; anything else — including one trace
/// running out before the other — is [`DivergenceReport::Diverged`].
pub fn compare(sim: &ObservationTrace, silicon: &ObservationTrace) -> DivergenceReport {
    let len = sim.checkpoints.len().max(silicon.checkpoints.len());

    for i in 0..len {
        let sim_cp = sim.checkpoints.get(i);
        let silicon_cp = silicon.checkpoints.get(i);

        match (sim_cp, silicon_cp) {
            (Some(s), Some(h)) => {
                // Decoded mismatch first: it's the friendlier observable
                // name. Checked ahead of the raw-event comparison because a
                // differing decoded value always implies differing raw
                // bytes too — reporting the variable name beats reporting
                // "itm_event" with an opaque byte dump whenever we can.
                if s.decoded != h.decoded {
                    let observable = s
                        .decoded
                        .as_ref()
                        .or(h.decoded.as_ref())
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| "itm_event".to_string());
                    return DivergenceReport::Diverged {
                        first_checkpoint: i,
                        observable,
                        sim_value: decoded_display(&s.decoded),
                        silicon_value: decoded_display(&h.decoded),
                    };
                }
                if s.itm_event != h.itm_event {
                    return DivergenceReport::Diverged {
                        first_checkpoint: i,
                        observable: "itm_event".to_string(),
                        sim_value: format!("{:?}", s.itm_event),
                        silicon_value: format!("{:?}", h.itm_event),
                    };
                }
            }
            (Some(s), None) => {
                return DivergenceReport::Diverged {
                    first_checkpoint: i,
                    observable: "itm_event".to_string(),
                    sim_value: format!("{:?}", s.itm_event),
                    silicon_value: "<no event>".to_string(),
                };
            }
            (None, Some(h)) => {
                return DivergenceReport::Diverged {
                    first_checkpoint: i,
                    observable: "itm_event".to_string(),
                    sim_value: "<no event>".to_string(),
                    silicon_value: format!("{:?}", h.itm_event),
                };
            }
            (None, None) => unreachable!("i < len means at least one side has a checkpoint"),
        }
    }

    DivergenceReport::Agreement {
        checkpoints_compared: len,
    }
}

fn decoded_display(decoded: &Option<(String, Value)>) -> String {
    match decoded {
        Some((_, value)) => value.to_string(),
        None => "<undecoded>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(port: u8, data: &[u8]) -> ItmEvent {
        ItmEvent {
            port,
            data: data.to_vec(),
        }
    }

    fn checkpoint(port: u8, data: &[u8], decoded: Option<(&str, Value)>) -> Checkpoint {
        Checkpoint {
            itm_event: event(port, data),
            decoded: decoded.map(|(name, value)| (name.to_string(), value)),
        }
    }

    #[test]
    fn identical_traces_agree() {
        let trace = ObservationTrace {
            checkpoints: vec![
                checkpoint(0, b"O", None),
                checkpoint(1, &[10, 0, 0, 0], Some(("speed", Value::from(10)))),
            ],
        };
        let report = compare(&trace, &trace.clone());
        assert_eq!(
            report,
            DivergenceReport::Agreement {
                checkpoints_compared: 2
            }
        );
    }

    #[test]
    fn differing_decoded_value_diverges_with_variable_name() {
        let sim = ObservationTrace {
            checkpoints: vec![
                checkpoint(0, b"O", None),
                checkpoint(1, &[10, 0, 0, 0], Some(("speed", Value::from(10)))),
            ],
        };
        let silicon = ObservationTrace {
            checkpoints: vec![
                checkpoint(0, b"O", None),
                checkpoint(1, &[12, 0, 0, 0], Some(("speed", Value::from(12)))),
            ],
        };
        let report = compare(&sim, &silicon);
        assert_eq!(
            report,
            DivergenceReport::Diverged {
                first_checkpoint: 1,
                observable: "speed".to_string(),
                sim_value: "10".to_string(),
                silicon_value: "12".to_string(),
            }
        );
    }

    #[test]
    fn differing_raw_itm_event_diverges_as_itm_event() {
        let sim = ObservationTrace {
            checkpoints: vec![checkpoint(0, b"O", None)],
        };
        let silicon = ObservationTrace {
            checkpoints: vec![checkpoint(0, b"K", None)],
        };
        let report = compare(&sim, &silicon);
        match report {
            DivergenceReport::Diverged {
                first_checkpoint,
                observable,
                ..
            } => {
                assert_eq!(first_checkpoint, 0);
                assert_eq!(observable, "itm_event");
            }
            other => panic!("expected Diverged, got {other:?}"),
        }
    }

    #[test]
    fn shorter_trace_diverges_at_its_length_with_no_event_marker() {
        let sim = ObservationTrace {
            checkpoints: vec![checkpoint(0, b"O", None), checkpoint(0, b"K", None)],
        };
        let silicon = ObservationTrace {
            checkpoints: vec![checkpoint(0, b"O", None)],
        };
        let report = compare(&sim, &silicon);
        assert_eq!(
            report,
            DivergenceReport::Diverged {
                first_checkpoint: 1,
                observable: "itm_event".to_string(),
                sim_value: format!("{:?}", event(0, b"K")),
                silicon_value: "<no event>".to_string(),
            }
        );
    }

    #[test]
    fn decode_finds_configured_variable() {
        let mut vars = VariableMap::new();
        vars.insert(1, "speed", nucleus_trace::translate::VarType::U32);
        let decoded = decode(&vars, &event(1, &[42, 0, 0, 0]));
        assert_eq!(decoded, Some(("speed".to_string(), Value::from(42))));
    }

    #[test]
    fn decode_returns_none_for_unconfigured_port() {
        let vars = VariableMap::new();
        let decoded = decode(&vars, &event(0, b"log line"));
        assert_eq!(decoded, None);
    }
}
