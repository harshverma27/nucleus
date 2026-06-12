# Nucleus — VS Code extension

A **thin client** for the Nucleus toolchain. It contains zero business logic: all
diagnostics, hover, and completion come from the Rust language server.

## Status (Phase 4 — LSP client complete)

- Activates on `workspaceContains:**/stm32.toml`.
- Spawns `nucleus lsp` (path configurable via `nucleus.serverPath`) and connects
  `vscode-languageclient` over stdio against `**/stm32.toml`.
- Surfaces the server's diagnostics (red squiggles on pin conflicts), pin hover,
  and pin-name completion in the editor.

The React trace dashboard webview (`src/dashboard/`) lands in Phase 6.

## Build

```sh
npm install
npm run build      # esbuild bundle -> dist/extension.js
npm run typecheck  # tsc --noEmit
```

> The extension is **not** built by the Rust CI gate (`make check`); it requires
> `npm install`. The Rust language server it talks to is fully tested in CI.

Requires the `nucleus` CLI on `PATH` (or set `nucleus.serverPath`).
