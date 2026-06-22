// Shared types for the Nucleus sidebar.
//
// Mirrors nucleus_history::{RunSummary, HistorySummary} from
// `nucleus history --graph`; the webview only renders it (Rust does the
// counting). Keeping this in lockstep with the Rust serialization is the only
// coupling between the CLI and the dashboard.

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
