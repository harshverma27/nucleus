// Dashboard entry point. Works unchanged in a VS Code webview and a standalone
// browser: both load this bundle and connect to the trace WebSocket. The host
// and port can be overridden with `?host=` / `?ws=` query params, or by a
// `window.NUCLEUS_WS_URL` injected by the webview host.

import React from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./dashboard.css";

declare global {
  interface Window {
    NUCLEUS_WS_URL?: string;
  }
}

function resolveWsUrl(): string {
  if (window.NUCLEUS_WS_URL) {
    return window.NUCLEUS_WS_URL;
  }
  const params = new URLSearchParams(window.location.search);
  const host = params.get("host") ?? "localhost";
  const port = params.get("ws") ?? "7878";
  return `ws://${host}:${port}`;
}

const container = document.getElementById("root");
if (container) {
  createRoot(container).render(<App wsUrl={resolveWsUrl()} />);
}
