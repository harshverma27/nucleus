# Nucleus — VS Code extension

A **thin client** for the Nucleus toolchain. It contains zero business logic: all
diagnostics, hover, completion, and CLI work happen in the Rust `nucleus` binary.

## What it does

**LSP client**
- Activates on `workspaceContains:**/stm32.toml`.
- Spawns `nucleus lsp` (path configurable via `nucleus.serverPath`) and connects
  `vscode-languageclient` over stdio against `**/stm32.toml`.
- Surfaces the server's diagnostics (red squiggles on pin conflicts), pin hover,
  and pin-name completion in the editor.

**Sidebar** (Activity Bar → "Nucleus")
- Four action buttons — **Check / Build / Flash / Test** — each runs the matching
  `nucleus <verb>` in an integrated terminal (live, colored output). The buttons
  are pure CLI spawns; no logic lives in the extension.
- A **test-history** bar chart (`src/dashboard/`, React + Canvas) rendered from
  `nucleus history --graph`, with a Refresh button.

## Build

```sh
npm install
npm run build      # esbuild bundle -> dist/extension.js
npm run typecheck  # tsc --noEmit
```

> The extension is **not** built by the Rust CI gate (`make check`); it requires
> `npm install`. The Rust language server it talks to is fully tested in CI.

Requires the `nucleus` CLI on `PATH` (or set `nucleus.serverPath`).
