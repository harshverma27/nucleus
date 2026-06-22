// Sidebar entry point. Runs inside the VS Code webview view: it grabs the host
// API, renders the button panel + history chart, and re-renders whenever the
// extension posts fresh history (`{type:"history", data}`).

import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";

import { Host, Sidebar } from "./Sidebar";
import { HistorySummary } from "./types";
import "./dashboard.css";

declare global {
  interface Window {
    acquireVsCodeApi?: () => Host;
  }
}

const host: Host = window.acquireVsCodeApi
  ? window.acquireVsCodeApi()
  : { postMessage: () => undefined };

function Root(): JSX.Element {
  const [history, setHistory] = useState<HistorySummary | undefined>();

  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.data?.type === "history") {
        setHistory(e.data.data as HistorySummary);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  return <Sidebar host={host} history={history} />;
}

const container = document.getElementById("root");
if (container) {
  createRoot(container).render(<Root />);
}
