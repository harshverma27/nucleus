# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Nucleus is currently **scaffolding only**. The `crates/`, `extension/`, `xtask/`, and `tests/` directories contain `README.md` placeholders describing intent — there is no `Cargo.toml`, no Rust source, and no TypeScript source yet. When asked to "build" or "run" something that doesn't exist, the task is usually to *create* it per the design in `README.md`, not to find existing code. Verify whether a crate/file actually exists before assuming.

`README.md` (root) is the authoritative product spec and roadmap. Read it before doing design or implementation work — it defines the component boundaries, the `stm32.toml` format, the 8-phase roadmap (each phase gated by measurable exit criteria), and known hard problems. `tasks.txt` holds the current Week 1 task breakdown for the Phase 1 constraint database.

## What Nucleus is

A CLI-first STM32 developer platform with two halves:

- **Rust CLI (`nucleus`)** — owns *all* logic: TOML parsing, the hardware constraint solver, HAL code generation, the LSP server, the ITM/CoreSight packet decoder, and OpenOCD/WebSocket trace plumbing.
- **VS Code extension (TypeScript + React)** — a thin display layer only: an LSP client and a webview hosting the React trace dashboard.

## Architectural rules (do not violate)

These come directly from the README's "Scope Discipline Rules" and codegen strategy. They are binding constraints, not suggestions:

1. **The extension contains zero business logic.** Any constraint checking, decoding, or validation belongs in the Rust CLI. If tempted to add logic to TypeScript, stop — it goes in a crate.
2. **The codegen does not reimplement the HAL.** Generated `nucleus_init.c` calls standard ST HAL `Init` functions (`HAL_UART_Init`, etc.) with resolved parameters via typed config structs in `nucleus_config.h`. Keep generated calls to `Init` functions only so ST HAL updates don't break us.
3. **One MCU through Phase 7: NUCLEO-F446RE.** The second MCU family (STM32L476RG) lands in Phase 8 to prove the DB design generalizes. (Note: README Week 1 text references the F411 pack as the parsing source — the vendored CMSIS pack ships both F411 and F446 headers.)
4. **No DMA collision detection through Phase 7. No full clock-tree solver (not scheduled).** The only clock validation is basic clock-domain checking ("is the peripheral's bus clock enabled?"), which ships in Phase 2.
5. **Local tool only** — no cloud registry, no upload features.
6. **The published-toolchain milestone is Phase 7:** `cargo install` + VS Code Marketplace + GitHub Actions release automation (cross-platform binaries, crate publish, `.vsix` upload on tag). This distribution/CI-CD work is the headline outcome — treat it as a first-class phase, not an afterthought.

## Workspace layout (intended)

Multi-crate Cargo workspace. Each crate is independently testable; shared logic lives in lib crates and `nucleus-cli` is the only binary:

- `nucleus-cli` — binary; command dispatch, orchestrates compiler/build/trace.
- `nucleus-compiler` — TOML parser, constraint solver, codegen.
- `nucleus-db` — STM32 constraint database; build-time pack parsing/normalization; pin/AF/peripheral lookup. The DB is embedded at compile time (via `build.rs` or `xtask`) and must produce **deterministic output** for testable CI.
- `nucleus-lsp` — `tower-lsp` server wrapping `nucleus-compiler` (diagnostics + hover).
- `nucleus-itm` — hand-rolled ARM CoreSight ITM packet decoder. Must never panic on malformed input (handle packets spanning read boundaries, overflow packets, resync after dropped connection); fuzz-test it.
- `nucleus-trace` — OpenOCD telnet integration + `tokio-tungstenite` WebSocket server (port 7878).
- `xtask` — developer automation (pack parsing, codegen, CI helpers), invoked as `cargo xtask`.
- `extension/` — VS Code extension; React dashboard under `extension/src/dashboard/`, bundled by esbuild.

## Vendored data source

`cmsis-device-f4-2.6.11/` is ST's official CMSIS device package for the STM32F4 family, vendored in as the **source of truth for the constraint database** (`nucleus-db`). Relevant device headers include `Include/stm32f411xe.h` and `Include/stm32f446xx.h`. Treat this as read-only upstream data — do not hand-edit it; corrections to pack data belong in a patch table inside `nucleus-db` (the README flags pack-data inconsistency as the biggest time sink).

## Commands

No build tooling exists yet. Once the workspace is created, the conventional commands will be:

- Build: `cargo build` (workspace) / `cargo build -p <crate>`
- Test: `cargo test` / `cargo test -p <crate>` / single test: `cargo test -p <crate> <test_name>`
- DB/codegen helpers: `cargo xtask <command>`
- Extension: `npm install` then `npm run build` (esbuild) inside `extension/`

The CLI surface (per README) is `nucleus init | check | build | flash | trace | lsp`. `nucleus check` must exit non-zero on any conflict so CI can gate on it.
