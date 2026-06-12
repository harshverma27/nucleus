// The dashboard shell: connection lifecycle, the render clock, theme, and the
// three resizable panels (log | variables / cpu-load). Identical in a VS Code
// webview and a standalone browser — both just point it at the trace WebSocket.

import React, { useEffect, useMemo, useState } from "react";

import { CpuLoadPanel } from "./CpuLoadPanel";
import { LogPanel } from "./LogPanel";
import { SplitPane } from "./SplitPane";
import { TraceStore } from "./store";
import { useTick } from "./useTick";
import { VariableChart } from "./VariableChart";

export function App({ wsUrl }: { wsUrl: string }): JSX.Element {
  const store = useMemo(() => new TraceStore(wsUrl), [wsUrl]);
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const tick = useTick(30);

  useEffect(() => store.connect(), [store]);

  return (
    <div className={`app theme-${theme}`}>
      <header className="app-header">
        <span className="brand">Nucleus Trace</span>
        <StatusDot status={store.status} />
        <span className="ws-url">{wsUrl}</span>
        {store.overflow > 0 && (
          <span className="overflow-badge" title="ITM FIFO overflows">
            ⚠ {store.overflow} overflow{store.overflow === 1 ? "" : "s"}
          </span>
        )}
        <span className="spacer" />
        <button onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
          {theme === "dark" ? "☀ Light" : "🌙 Dark"}
        </button>
      </header>

      <div className="app-body">
        <SplitPane direction="horizontal" initial={40}>
          <LogPanel store={store} />
          <SplitPane direction="vertical" initial={65}>
            <VariableChart store={store} tick={tick} />
            <CpuLoadPanel store={store} tick={tick} />
          </SplitPane>
        </SplitPane>
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
