# Trace dashboard (React + Canvas)

The real-time ITM trace UI. A single bundle that runs **identically** in a VS
Code webview and a standalone browser — both connect to the `nucleus trace`
WebSocket (`ws://localhost:7878` by default).

## Status (Phase 6 — complete)

- **Log panel** (`LogPanel.tsx`) — port-0 output, timestamped, with live
  search/filter, follow-tail, clear, and **export as text**.
- **Variable timeline** (`VariableChart.tsx`) — a live Canvas line chart of up
  to 7 traced variables (ports 1–7) with an auto-scaling Y axis and a rolling
  30 s window; a legend shows current values.
- **CPU-load strip** (`CpuLoadPanel.tsx`) — a filled Canvas strip chart of the
  utilization estimate the daemon derives from DWT PC-sampling packets.
- **Polish** — resizable panels (`SplitPane.tsx`), dark/light theme toggle,
  connection status, overflow badge.

Data flows through a plain `TraceStore` (mutated in place at the trace data
rate) and the UI re-reads it on an animation-frame tick (`useTick`), so
sub-millisecond data rates don't drown React in re-renders. Buffers are capped.

`types.ts`'s `TraceEvent` mirrors `nucleus-trace::translate::TraceEvent` — the
only coupling between daemon and dashboard.

## Build

Bundled by esbuild from the extension root:

```sh
cd extension
npm install
npm run build         # -> dist/dashboard.js, dist/dashboard.css, dist/index.html
```

Open `dist/index.html` in a browser for the standalone view, or run
**Nucleus: Open Trace Dashboard** in VS Code for the webview. Either way, start
`nucleus trace` first.
