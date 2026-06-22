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

// History mode (M9). Mirrors nucleus_ledger::trend::{TrendData, TrendPoint,
// BackendCounts}. Produced by `nucleus history --graph`; the dashboard only
// renders it (all counting happens in Rust).

export interface BackendCounts {
  backend: string;
  pass: number;
  fail: number;
  skip: number;
}

export interface TrendPoint {
  hash: string;
  short: string;
  timestamp: number;
  family: string;
  approved: boolean;
  conflicts: number;
  backends: BackendCounts[];
  pass: number;
  fail: number;
  skip: number;
}

export interface TrendData {
  schema: string;
  points: TrendPoint[];
}
