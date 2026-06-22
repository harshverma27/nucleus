# Sidebar dashboard (React + Canvas)

The webview rendered inside the **Nucleus** Activity Bar view. A single esbuild
bundle (`dist/dashboard.js` + `dashboard.css`) hosted by `src/panelView.ts`.

## What it renders

- **Action buttons** (`Sidebar.tsx`) — Check / Build / Flash / Test. Each posts
  `{type:"run", verb}` to the extension host, which runs `nucleus <verb>` in a
  terminal. No CLI logic here — the host owns the spawn (and whitelists the verb).
- **Test-history chart** (`HistoryPanel.tsx`) — a per-run pass/fail stacked-bar
  Canvas chart with a "last N" filter and JSON export. The counts come
  pre-computed from `nucleus history --graph`; this panel only draws them.

## Data flow

`index.tsx` grabs the VS Code webview API (`acquireVsCodeApi`), renders
`<Sidebar>`, and listens for `{type:"history", data}` messages from the host
(pushed on open and on Refresh). `types.ts` mirrors
`nucleus_history::{RunSummary, HistorySummary}` — the only coupling between the
CLI and the dashboard.

## Build

Bundled by esbuild from the extension root:

```sh
cd extension
npm install
npm run build         # -> dist/dashboard.js, dist/dashboard.css, dist/index.html
```

The real host is the VS Code sidebar; `dist/index.html` is a standalone fallback
(no history feed, so it just shows the buttons).
