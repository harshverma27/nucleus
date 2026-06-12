# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

**Phases 1, 2, and 3 are complete.**

Phase 1: `crates/nucleus-db` — a pin/AF/peripheral model with lookup APIs, generated at build time (`build.rs`) from ST's open pin data XML vendored in `crates/nucleus-db/packdata/`. The full F446RE table (~275 mappings across 45 GPIOs) is byte-deterministic and cross-validated by a unit test against a hand-verified datasheet seed (`src/data.rs::SEED`).

Phase 2: `crates/nucleus-compiler` (`stm32.toml` parser + hardware constraint solver) and `crates/nucleus-cli` (the `nucleus` binary). `nucleus check <path>` parses a config, validates it against the F446RE database, prints conflicts, and exits non-zero on any error (so CI can gate on it). The solver detects all four Phase-2 conflict classes — pin collision, AF mismatch, missing required pins, clock-domain-disabled — each unit-tested; a CLI integration test (`crates/nucleus-cli/tests/cli.rs`) drives the binary against repo fixtures in `tests/fixtures/`, including the deliberate PA5-collision fixture that must produce exactly one error. Per the scope rules there is no DMA-collision detection and only basic "is the bus clock enabled?" clock checking (driven by an optional `[clocks]` section, default all-enabled).

Phase 3: HAL codegen in `nucleus-compiler` (`src/codegen.rs`) and the `nucleus init`/`build`/`flash` orchestration in `nucleus-cli`. `nucleus init` scaffolds a complete project (`src/scaffold.rs` templates: `stm32.toml`, `CMakeLists.txt`, `cmake/` cross-toolchain file, `src/main.c`, CI workflow, `.gitignore`) and is idempotent — it never overwrites existing files. `nucleus build` (`src/firmware.rs`) validates the config, refuses to generate code if there are conflicts, writes `src/generated/nucleus_config.h` and `nucleus_init.c`, then drives CMake + arm-none-eabi-gcc; a missing cross toolchain yields a clear error (codegen still runs and is observable). `nucleus flash` programs `build/firmware.bin` with `st-flash`. The generated `Nucleus_Init()` calls only stock `HAL_*_Init` functions with resolved params from typed config structs, and configures GPIO alternate-function muxing using AF numbers from `nucleus-db` (`GPIO_AF<n>_<PERIPH>` macros) — it does not reimplement the HAL.

The remaining crates (`nucleus-lsp`, `nucleus-itm`, `nucleus-trace`), the `extension/`, and `xtask/` are still `README.md` placeholders — no source yet. The `nucleus-cli` subcommands `trace`/`lsp` are declared but stubbed (they print a "scheduled for Phase N" notice and exit non-zero) until their phases land. When asked to "build" or "run" something that doesn't exist, the task is usually to *create* it per the design in `README.md`, not to find existing code.

Key `nucleus-compiler` facts: the peripheral model (`src/model.rs`) is a small hand-maintained table mapping `stm32.toml` keys (`tx`, `mosi`, `sda`, `channel1`…) to DB signal names, marking required vs. optional pins, and assigning each peripheral to an F446 bus (APB1/APB2/AHB1). Peripheral instance names map to DB names by upper-casing (`usart2` → `USART2`); the codegen handle suffix is the instance's **trailing** digit run (`i2c1` → `hi2c1`, never `hi2c21`). The solver returns conflicts in deterministic order; collisions are reported once per over-subscribed pin, not per pair. Codegen output is byte-deterministic. The "compiles/flashes on real hardware" exit criterion is a maintainer step that needs the ARM toolchain + an STM32CubeF4 HAL checkout (wired via `STM32CUBE_PATH` in the scaffolded CMake) + the physical NUCLEO-F446RE; CI verifies codegen structure, not the on-board flash.

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
