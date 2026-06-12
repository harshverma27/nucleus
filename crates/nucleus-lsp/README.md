# nucleus-lsp

The Nucleus language server: live `stm32.toml` feedback for any LSP editor.

## Status (Phase 4 — complete)

A [`tower-lsp`](https://crates.io/crates/tower-lsp) server, started by `nucleus lsp` over stdio.

- `analysis` — the real logic, as **pure, synchronous, unit-tested** functions:
  - `diagnostics(text)` — runs the compiler and maps every conflict to a source range (a collision underlines each colliding pin; missing-pin/clock conflicts underline the `[peripherals.…]` header; TOML/schema errors use the parser's span).
  - `hover(text, pos)` — the pin under the cursor → its full alternate-function table from `nucleus-db`.
  - `completion(text, pos)` — pin names, offered on a value line inside a peripherals table.
- `server` — a thin async shell: an in-memory document map (FULL text sync), `publishDiagnostics` on open/change, and hover/completion delegating to `analysis`.
- `run_stdio()` — the entry point the CLI calls; blocks on a Tokio runtime.

All hardware knowledge comes from `nucleus-compiler` / `nucleus-db`; this crate only translates between source spans and LSP ranges. The VS Code extension is a thin client that speaks to this server — no logic lives in TypeScript.
