// Shared types for the Nucleus trace dashboard.
//
// `TraceEvent` mirrors the JSON the `nucleus trace` WebSocket emits (see
// nucleus-trace::translate::TraceEvent). Keeping this in lockstep with the Rust
// serialization is the only coupling between the daemon and the dashboard.

export type TraceEvent =
  | { kind: "log"; message: string }
  | { kind: "variable"; port: number; name: string; type: string; value: number }
  | { kind: "overflow" }
  | { kind: "cpuload"; load: number };

export interface LogEntry {
  id: number;
  /** Wall-clock arrival time (ms since epoch). */
  t: number;
  message: string;
}

export interface Point {
  t: number;
  v: number;
}

export interface Series {
  name: string;
  port: number;
  type: string;
  color: string;
  points: Point[];
}

export type ConnectionStatus = "connecting" | "open" | "closed";

// History mode. Mirrors nucleus_history::{RunSummary, HistorySummary} from
// `nucleus history --graph`; the dashboard only renders it (Rust does the
// counting).

export interface RunSummary {
  timestamp: number;
  pass: number;
  fail: number;
  skip: number;
}

export interface HistorySummary {
  schema: string;
  runs: RunSummary[];
}
