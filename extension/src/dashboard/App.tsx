// The dashboard shell: connection lifecycle, the render clock, theme, and the
// three resizable panels (log | variables / cpu-load). Identical in a VS Code
// webview and a standalone browser — both just point it at the trace WebSocket.

import React, { useEffect, useMemo, useState } from "react";

import { CpuLoadPanel } from "./CpuLoadPanel";
import { HistoryPanel } from "./HistoryPanel";
import { LogPanel } from "./LogPanel";
import { SplitPane } from "./SplitPane";
import { TraceStore } from "./store";
import { HistorySummary } from "./types";
import { useTick } from "./useTick";
import { VariableChart } from "./VariableChart";

type Mode = "trace" | "history";

export function App({
  wsUrl,
  history,
}: {
  wsUrl: string;
  history?: HistorySummary;
}): JSX.Element {
  const store = useMemo(() => new TraceStore(wsUrl), [wsUrl]);
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  // Start in history mode when the host injected history data, otherwise live trace.
  const [mode, setMode] = useState<Mode>(history ? "history" : "trace");
  const tick = useTick(30);

  // Only hold the live trace connection open while the trace view is active.
  useEffect(() => {
    if (mode === "trace") return store.connect();
  }, [store, mode]);

  return (
    <div className={`app theme-${theme}`}>
      <header className="app-header">
        <span className="brand">Nucleus</span>
        {history && (
          <span className="mode-toggle">
            <button
              className={mode === "trace" ? "active" : ""}
              onClick={() => setMode("trace")}
            >
              📡 Trace
            </button>
            <button
              className={mode === "history" ? "active" : ""}
              onClick={() => setMode("history")}
            >
              📈 History
            </button>
          </span>
        )}
        {mode === "trace" && (
          <>
            <StatusDot status={store.status} />
            <span className="ws-url">{wsUrl}</span>
            {store.overflow > 0 && (
              <span className="overflow-badge" title="ITM FIFO overflows">
                ⚠ {store.overflow} overflow{store.overflow === 1 ? "" : "s"}
              </span>
            )}
          </>
        )}
        <span className="spacer" />
        <button onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
          {theme === "dark" ? "☀ Light" : "🌙 Dark"}
        </button>
      </header>

      <div className="app-body">
        {mode === "history" && history ? (
          <HistoryPanel summary={history} />
        ) : (
          <SplitPane direction="horizontal" initial={40}>
            <LogPanel store={store} />
            <SplitPane direction="vertical" initial={65}>
              <VariableChart store={store} tick={tick} />
              <CpuLoadPanel store={store} tick={tick} />
            </SplitPane>
          </SplitPane>
        )}
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: TraceStore["status"] }): JSX.Element {
  const label =
    status === "open" ? "connected" : status === "connecting" ? "connecting…" : "disconnected";
  return (
    <span className={`status status-${status}`} title={label}>
      <span className="status-dot" />
      {label}
    </span>
  );
}
