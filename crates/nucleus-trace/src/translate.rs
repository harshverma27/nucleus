//! Turning raw ITM [`Packet`]s into named, typed [`TraceEvent`]s.
//!
//! This is the only place trace *meaning* is assigned: stimulus port 0 is the
//! UTF-8 log stream (it replaces `printf`), and ports 1–7 carry typed variables
//! named by the project's `[[trace.variables]]` table. The mapping is pure and
//! unit-tested; the async plumbing in [`crate::server`] just forwards results.

use std::collections::BTreeMap;

use nucleus_itm::Packet;
use serde::Serialize;

/// The numeric type a traced variable's bytes decode to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarType {
    F32,
    U16,
    U32,
    I32,
}

impl VarType {
    /// Parse the `type = "…"` field of a `[[trace.variables]]` entry.
    pub fn parse(s: &str) -> Option<VarType> {
        match s {
            "f32" => Some(VarType::F32),
            "u16" => Some(VarType::U16),
            "u32" => Some(VarType::U32),
            "i32" => Some(VarType::I32),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            VarType::F32 => "f32",
            VarType::U16 => "u16",
            VarType::U32 => "u32",
            VarType::I32 => "i32",
        }
    }

    /// Decode little-endian `data` to a JSON number, zero-padding or truncating
    /// to the type's width so malformed widths never panic.
    fn decode(self, data: &[u8]) -> serde_json::Value {
        let mut buf4 = [0u8; 4];
        let n = data.len().min(4);
        buf4[..n].copy_from_slice(&data[..n]);
        match self {
            VarType::F32 => json_f64(f64::from(f32::from_le_bytes(buf4))),
            VarType::U32 => serde_json::Value::from(u32::from_le_bytes(buf4)),
            VarType::I32 => serde_json::Value::from(i32::from_le_bytes(buf4)),
            VarType::U16 => {
                let mut buf2 = [0u8; 2];
                let m = data.len().min(2);
                buf2[..m].copy_from_slice(&data[..m]);
                serde_json::Value::from(u16::from_le_bytes(buf2))
            }
        }
    }
}

fn json_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

/// One traced variable: its name and decode type.
#[derive(Debug, Clone)]
pub struct Variable {
    pub name: String,
    pub ty: VarType,
}

/// Port → variable mapping, built from `[[trace.variables]]`.
#[derive(Debug, Clone, Default)]
pub struct VariableMap(BTreeMap<u8, Variable>);

impl VariableMap {
    pub fn new() -> VariableMap {
        VariableMap(BTreeMap::new())
    }

    /// Register `name`/`ty` on stimulus `port`.
    pub fn insert(&mut self, port: u8, name: impl Into<String>, ty: VarType) {
        self.0.insert(
            port,
            Variable {
                name: name.into(),
                ty,
            },
        );
    }

    /// Build a map from a parsed `[trace]` config, ignoring entries with an
    /// unknown type (port 0 is always the log stream and needs no entry).
    pub fn from_config(trace: &nucleus_compiler::config::Trace) -> VariableMap {
        let mut map = VariableMap::new();
        for v in &trace.variables {
            if let Some(ty) = VarType::parse(&v.ty) {
                map.insert(v.port, v.name.clone(), ty);
            }
        }
        map
    }

    fn get(&self, port: u8) -> Option<&Variable> {
        self.0.get(&port)
    }
}

/// A structured trace event, serialized to JSON for WebSocket clients.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TraceEvent {
    /// A decoded line from the port-0 log stream.
    Log { message: String },
    /// A typed variable update from ports 1–7.
    Variable {
        port: u8,
        name: String,
        #[serde(rename = "type")]
        ty: &'static str,
        value: serde_json::Value,
    },
    /// The device reported a trace FIFO overflow.
    Overflow,
}

/// Stateful translator: maps a stream of [`Packet`]s to [`TraceEvent`]s,
/// reassembling port-0 bytes into whole log lines.
#[derive(Debug)]
pub struct Translator {
    vars: VariableMap,
    /// Bytes received on port 0 since the last newline.
    log_line: Vec<u8>,
}

impl Translator {
    pub fn new(vars: VariableMap) -> Translator {
        Translator {
            vars,
            log_line: Vec::new(),
        }
    }

    /// Translate one packet into zero or more events.
    pub fn translate(&mut self, packet: &Packet) -> Vec<TraceEvent> {
        match packet {
            Packet::Instrumentation { port: 0, data } => self.push_log_bytes(data),
            Packet::Instrumentation { port, data } => match self.vars.get(*port) {
                Some(var) => vec![TraceEvent::Variable {
                    port: *port,
                    name: var.name.clone(),
                    ty: var.ty.name(),
                    value: var.ty.decode(data),
                }],
                None => Vec::new(), // unmapped port: nothing to report
            },
            Packet::Overflow => vec![TraceEvent::Overflow],
            // Timestamps, sync, hardware, extension carry no user-facing value yet.
            _ => Vec::new(),
        }
    }

    /// Flush any buffered partial log line (e.g. on shutdown).
    pub fn flush(&mut self) -> Vec<TraceEvent> {
        if self.log_line.is_empty() {
            return Vec::new();
        }
        let message = take_utf8(&mut self.log_line);
        vec![TraceEvent::Log { message }]
    }

    fn push_log_bytes(&mut self, data: &[u8]) -> Vec<TraceEvent> {
        let mut events = Vec::new();
        for &b in data {
            if b == b'\n' {
                let message = take_utf8(&mut self.log_line);
                events.push(TraceEvent::Log { message });
            } else if b != b'\r' {
                self.log_line.push(b);
            }
        }
        events
    }
}

/// Drain `buf` into a lossy-UTF-8 string.
fn take_utf8(buf: &mut Vec<u8>) -> String {
    let s = String::from_utf8_lossy(buf).into_owned();
    buf.clear();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> VariableMap {
        let mut m = VariableMap::new();
        m.insert(1, "temperature", VarType::F32);
        m.insert(2, "duty", VarType::U16);
        m.insert(3, "loop_us", VarType::U32);
        m
    }

    #[test]
    fn port0_bytes_become_log_lines() {
        let mut t = Translator::new(map());
        let mut events = Vec::new();
        for ch in b"hi\nbye\n" {
            events.extend(t.translate(&Packet::Instrumentation {
                port: 0,
                data: vec![*ch],
            }));
        }
        assert_eq!(
            events,
            vec![
                TraceEvent::Log {
                    message: "hi".into()
                },
                TraceEvent::Log {
                    message: "bye".into()
                },
            ]
        );
    }

    #[test]
    fn partial_log_line_waits_for_newline() {
        let mut t = Translator::new(map());
        assert!(t
            .translate(&Packet::Instrumentation {
                port: 0,
                data: b"par".to_vec()
            })
            .is_empty());
        let done = t.translate(&Packet::Instrumentation {
            port: 0,
            data: b"tial\n".to_vec(),
        });
        assert_eq!(
            done,
            vec![TraceEvent::Log {
                message: "partial".into()
            }]
        );
    }

    #[test]
    fn f32_variable_decodes_to_a_number() {
        let mut t = Translator::new(map());
        let events = t.translate(&Packet::Instrumentation {
            port: 1,
            data: 3.5f32.to_le_bytes().to_vec(),
        });
        assert_eq!(
            events,
            vec![TraceEvent::Variable {
                port: 1,
                name: "temperature".into(),
                ty: "f32",
                value: serde_json::json!(3.5),
            }]
        );
    }

    #[test]
    fn u16_and_u32_variables_decode() {
        let mut t = Translator::new(map());
        let duty = t.translate(&Packet::Instrumentation {
            port: 2,
            data: 1000u16.to_le_bytes().to_vec(),
        });
        assert_eq!(
            duty[0],
            TraceEvent::Variable {
                port: 2,
                name: "duty".into(),
                ty: "u16",
                value: serde_json::json!(1000),
            }
        );
        let lt = t.translate(&Packet::Instrumentation {
            port: 3,
            data: 70_000u32.to_le_bytes().to_vec(),
        });
        assert_eq!(
            lt[0],
            TraceEvent::Variable {
                port: 3,
                name: "loop_us".into(),
                ty: "u32",
                value: serde_json::json!(70_000),
            }
        );
    }

    #[test]
    fn unmapped_port_is_ignored() {
        let mut t = Translator::new(map());
        assert!(t
            .translate(&Packet::Instrumentation {
                port: 5,
                data: vec![1, 2, 3, 4]
            })
            .is_empty());
    }

    #[test]
    fn overflow_is_reported_and_serializes() {
        let mut t = Translator::new(map());
        let events = t.translate(&Packet::Overflow);
        assert_eq!(events, vec![TraceEvent::Overflow]);
        let json = serde_json::to_string(&events[0]).unwrap();
        assert_eq!(json, r#"{"kind":"overflow"}"#);
    }

    #[test]
    fn variable_event_json_shape() {
        let ev = TraceEvent::Variable {
            port: 1,
            name: "temperature".into(),
            ty: "f32",
            value: serde_json::json!(3.5),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "variable",
                "port": 1,
                "name": "temperature",
                "type": "f32",
                "value": 3.5
            })
        );
    }
}
