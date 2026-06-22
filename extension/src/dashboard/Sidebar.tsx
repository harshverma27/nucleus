// The whole sidebar UI: a row of action buttons plus the test-history chart.
// Zero logic — buttons post a message to the extension host, which runs the
// matching `nucleus <verb>` in a terminal. History is pushed in from the host.

import React from "react";

import { HistoryPanel } from "./HistoryPanel";
import { HistorySummary } from "./types";

export interface Host {
  postMessage(msg: unknown): void;
}

const ACTIONS: { verb: string; label: string; title: string }[] = [
  { verb: "check", label: "Check", title: "nucleus check — run the constraint solver" },
  { verb: "build", label: "Build", title: "nucleus build — codegen + compile" },
  { verb: "flash", label: "Flash", title: "nucleus flash — program the board" },
  { verb: "test", label: "Test", title: "nucleus test — run HIL tests" },
];

export function Sidebar({
  host,
  history,
}: {
  host: Host;
  history?: HistorySummary;
}): JSX.Element {
  return (
    <div className="sidebar">
      <div className="nucleus-actions">
        {ACTIONS.map((a) => (
          <button
            key={a.verb}
            title={a.title}
            onClick={() => host.postMessage({ type: "run", verb: a.verb })}
          >
            {a.label}
          </button>
        ))}
      </div>

      <div className="history-wrap">
        <div className="history-bar">
          <span className="section-title">Test history</span>
          <span className="spacer" />
          <button
            title="Re-run nucleus history --graph"
            onClick={() => host.postMessage({ type: "refreshHistory" })}
          >
            ⟳ Refresh
          </button>
        </div>
        {history ? (
          <HistoryPanel summary={history} />
        ) : (
          <div className="empty">loading history…</div>
        )}
      </div>
    </div>
  );
}
