# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

**Phase 1 is complete.** The Cargo workspace exists with one real crate, `crates/nucleus-db`: a pin/AF/peripheral model with lookup APIs, generated at build time (`build.rs`) from ST's open pin data XML vendored in `crates/nucleus-db/packdata/`. The full F446RE table (~275 mappings across 45 GPIOs) is byte-deterministic and cross-validated by a unit test against a hand-verified datasheet seed (`src/data.rs::SEED`). The remaining crates (`nucleus-compiler`, `nucleus-cli`, `nucleus-lsp`, `nucleus-itm`, `nucleus-trace`), the `extension/`, and `xtask/` are still `README.md` placeholders — no source yet. When asked to "build" or "run" something that doesn't exist, the task is usually to *create* it per the design in `README.md`, not to find existing code.

Key `nucleus-db` facts: one (pin, AF) can carry **multiple** signals (SPI/I2S share AF numbers), so `Database::lookup` returns an iterator, not an `Option`. `src/pack.rs` is deliberately self-contained (no `crate::` types) because `build.rs` includes it via `#[path]`. Upstream data anomalies are never fixed by editing `packdata/` — structural ones are normalized in the parser (documented at `pack::PATCHES`), per-entry ones go in the `PATCHES` table.

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

## Vendored data sources

- `crates/nucleus-db/packdata/` — ST's open pin data XML (from [STM32_open_pin_data](https://github.com/STMicroelectronics/STM32_open_pin_data)): the MCU package pinout and the GPIO AF mux tables. **This is the source of truth for the constraint database**; `build.rs` parses it at build time. See `packdata/README.md`.
- `cmsis-device-f4-2.6.11/` — ST's CMSIS device package (register/startup headers, e.g. `Include/stm32f446xx.h`). Carries **no** pin↔AF mux data; kept for future HAL/register work (codegen, Phase 3).

Both are read-only upstream data — never hand-edit them; corrections to pack data belong in the patch table in `nucleus-db/src/pack.rs` (the README flags pack-data inconsistency as the biggest time sink).

## Commands

The `Makefile` is the task runner; `make` targets and CI run the identical checks.

- Full local gate (run before pushing): `make check` (= `fmt-check` + `lint` + `test`)
- Build: `make build` / `cargo build -p <crate>`
- Test: `make test` / `cargo test -p <crate>` / single test: `cargo test -p <crate> <test_name>`
- Format / lint: `make fmt` (apply) · `make fmt-check` · `make lint` (`clippy -D warnings`)
- DB/codegen helpers (future): `cargo xtask <command>`
- Extension (future): `npm install` then `npm run build` (esbuild) inside `extension/`

CI lives in `.github/workflows/`: `ci.yml` runs the gate + a cross-platform build matrix on every PR; `release.yml` is the Phase 7 skeleton that builds cross-platform artifacts on a `v*` tag.

The CLI surface (per README) is `nucleus init | check | build | flash | trace | lsp`. `nucleus check` must exit non-zero on any conflict so CI can gate on it.
