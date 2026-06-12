// The log stream panel: timestamped port-0 output, with live search/filter and
// "export as text". Replaces the serial monitor for debug logging.

import React, { useMemo, useState } from "react";

import { TraceStore } from "./store";
import { download, formatTime } from "./util";

export function LogPanel({ store }: { store: TraceStore }): JSX.Element {
  const [query, setQuery] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);

  const needle = query.trim().toLowerCase();
  const visible = useMemo(
    () =>
      needle
        ? store.logs.filter((l) => l.message.toLowerCase().includes(needle))
        : store.logs,
    // store.logs is mutated in place; length is the cheap change signal.
    [needle, store.logs, store.logs.length]
  );

  const bottom = (el: HTMLDivElement | null) => {
    if (el && autoScroll) {
      el.scrollTop = el.scrollHeight;
    }
  };

  const exportText = () =>
    download(
      "nucleus-trace.log",
      store.logs.map((l) => `${formatTime(l.t)}  ${l.message}`).join("\n")
    );

  return (
    <section className="panel">
      <header className="panel-header">
        <h2>Log</h2>
        <input
          className="search"
          type="search"
          placeholder="filter…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="count">{visible.length}</span>
        <label className="toggle">
          <input
            type="checkbox"
            checked={autoScroll}
            onChange={(e) => setAutoScroll(e.target.checked)}
          />
          follow
        </label>
        <button onClick={exportText}>Export</button>
        <button onClick={() => store.clear()}>Clear</button>
      </header>
      <div className="log-list" ref={bottom}>
        {visible.map((l) => (
          <div className="log-row" key={l.id}>
            <span className="log-time">{formatTime(l.t)}</span>
            <span className="log-msg">{l.message}</span>
          </div>
        ))}
        {visible.length === 0 && (
          <div className="empty">no log output{needle ? " matching filter" : ""}</div>
        )}
      </div>
    </section>
  );
}
