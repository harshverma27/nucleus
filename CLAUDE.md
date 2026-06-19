# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

**v1 (Phases 1–8) shipped. v2 (M1–M10 verifier/HIL/lockstep) underway.**

v1 complete: `nucleus-db` (F446RE + F411RE pin/AF/peripheral, byte-deterministic) → `nucleus-compiler` + `nucleus-cli` (parser, 4-conflict solver) → HAL codegen + `init`/`build`/`flash` → `nucleus-lsp` (LSP server, diagnostics/hover) → `nucleus-itm` (zero-panic ITM decoder, fuzz-tested) + `nucleus-trace` (WebSocket, JSON events) → React/Canvas dashboard + DWT decoding → Phase 7 distribution (crates.io, GitHub Marketplace, cross-platform release automation).

v2 (current branch: `20-v2-week-1-verify-completion-prove-infrastructure`) adds a verifier + dual-backend HIL loop:

**M1 — Clock-tree solver** ✅ DONE
- Family-parameterized oscillator/PLL/prescaler model + frequency math
- Validates frequency against silicon limits (APB1 ≤ 45 MHz, APB2 ≤ 90 MHz, SYSCLK ≤ 180 MHz on F446)
- Rejects over-clocked buses, unreachable baud rates
- `Conflict::ClockConstraint` in solver, surfaces via LSP

**M2 — DMA arbitration** ✅ DONE
- DMA1/DMA2 streams × channels + peripheral-request map (from RM0390/RM0383)
- Detects stream collisions, suggests alternatives
- `Conflict::DmaCollision` in solver, surfaces via LSP

**M3–M10 in progress** (see GitHub issues #19 umbrella, #20 Week 1, #21 Week 2)
- M3: IRQ/NVIC verifier (EXTI shared lines, enabled-but-unhandled)
- M4: Constraint auto-router (CSP solver: declare intent, get pinout)
- M5: Dual-backend HIL substrate (QEMU + hardware backends)
- M6: Declarative tests (`[[test]]` blocks in stm32.toml)
- M7: Scripted tests (device agent + host SDK)
- M8: Project ledger (`.nucleus/` version store)
- M9: History graphs + CI-native HIL (PR summary with per-backend results)
- M10: Lockstep co-execution (detect sim↔silicon divergence)

See `docs/superpowers/specs/2026-06-17-nucleus-v2-design.md` for the full v2 thesis and architecture.

When asked to "build" or "run" something that doesn't exist, the task is usually to *create* it per the design in `README.md` v2 spec or the milestone issues, not to find existing code.

## What Nucleus is

A CLI-first STM32 developer platform with two halves:

- **Rust CLI (`nucleus`)** — owns *all* logic: TOML parsing, the hardware constraint solver, HAL code generation, the LSP server, the ITM/CoreSight packet decoder, and OpenOCD/WebSocket trace plumbing.
- **VS Code extension (TypeScript + React)** — a thin display layer only: an LSP client and a webview hosting the React trace dashboard.

## Architectural rules (do not violate)

These are binding constraints from README v1 + v2 design. Non-negotiable:

1. **The extension contains zero business logic.** Any constraint checking, decoding, validation belongs in the Rust CLI. If tempted to add logic to TypeScript, stop — it goes in a crate.
2. **The codegen does not reimplement the HAL.** Generated `nucleus_init.c` calls only stock `HAL_*_Init` (e.g. `HAL_UART_Init`) with resolved params via typed config structs in `nucleus_config.h`. Keep it to Init calls so ST HAL updates don't break us.
3. **Target MCUs: F446RE + F411RE.** Both shipped and supported end-to-end (two AF tables, family-aware lookups, family-specific constraints for clocks/DMA/IRQ). No new families in v2.
4. **v2 deliberately lifts v1 limits:** M1 (clock-tree solver) and M2 (DMA arbitration) are done. M3–M10 add IRQ, auto-routing, dual-backend HIL, and lockstep — this is the whole point of v2. See the v2 design spec.
5. **Local tool only** — no cloud registry, no upload features.
6. **Phase 7 (v1) distribution is complete:** `cargo install` works, extension on Marketplace, cross-platform release automation in place.

## Workspace layout

Multi-crate Cargo workspace. Each crate independently testable; shared logic in lib crates; `nucleus-cli` is the only binary.

**v1 + v2 shared crates:**
- `nucleus-cli` — binary; command dispatch, orchestrates compiler/build/trace/**test**/history/lockstep.
- `nucleus-compiler` — TOML parser, constraint solver (M1–M3 conflicts), codegen, `[[test]]` parsing (M6).
- `nucleus-db` — STM32 constraint database: pin/AF/peripheral lookup (v1), plus clock-tree (M1), DMA (M2), IRQ/NVIC (M3) models. Build-time-generated from vendored ST pack data; deterministic output for CI.
- `nucleus-lsp` — `tower-lsp` server wrapping compiler (diagnostics/hover/completion for config + `[[test]]` blocks).
- `nucleus-itm` — zero-dependency, never-panic ITM/CoreSight decoder. Reused by M5 HIL backends.
- `nucleus-trace` — OpenOCD telnet + WebSocket server. `src/source.rs` reused by M5 hardware backend.
- `xtask` — build helpers (future: pack generation, xtask commands).
- `extension/` — VS Code client; React dashboard (Phase 6, reused for M9 history graphs).

**v2 new crates:**
- `nucleus-hil` *(M5 onwards)* — host-side test runner; backend trait + QEMU + hardware backends; observation API; M10 lockstep comparator.
- `nucleus-ledger` *(M8 onwards)* — `.nucleus/` version store; content-addressed storage; query API for `history`/`show` verbs.

## Key facts for v2 work

**`nucleus-compiler` solver:** The `Conflict` enum carries M1–M3 variants: `ClockConstraint` (M1), `DmaCollision` (M2), `IrqConflict` (M3), plus v1's pin-related conflicts. All checked in `src/solver.rs`, deterministic order, exit non-zero on any. LSP (`nucleus-lsp::analysis.rs`) maps each conflict to the most relevant source span automatically.

**`nucleus-db` model layers:**
- v1: pin/AF/peripheral from pack XML (deterministic)
- M1: `src/clock.rs` — family-parameterized clock-tree model (hand-maintained, seed-tested against RM0390/RM0383)
- M2: `src/dma.rs` — DMA request map (hand-maintained, RM tables 28–29)
- M3: `src/irq.rs` — NVIC/EXTI model (hand-maintained, RM IRQ tables)

**New v2 crates:** `nucleus-hil` (M5+) owns dual-backend runner, observation API, lockstep. `nucleus-ledger` (M8+) owns version store under `.nucleus/`.

**Testing discipline:** v1 conflicts unit-tested. v2 adds:
- M1: hand-verified clock frequencies (both families)
- M2: DMA tables from RM (both families)
- M3: EXTI groupings from RM (both families)
- M4–M10: golden fixtures (parser, solver output) + HIL integration tests (QEMU always; hardware optional)

**README + issues:** `README.md` is authoritative spec. GitHub #19 (umbrella), #20 (Week 1: M3–M5), #21 (Week 2: M6–M10) track the work. `docs/superpowers/specs/2026-06-17-nucleus-v2-design.md` is the full thesis.

## Vendored data sources

- `crates/nucleus-db/packdata/` — ST's open pin data XML (from [STM32_open_pin_data](https://github.com/STMicroelectronics/STM32_open_pin_data)): the MCU package pinout and the GPIO AF mux tables. **This is the source of truth for the constraint database**; `build.rs` parses it at build time. See `packdata/README.md`.
- `cmsis-device-f4-2.6.11/` — ST's CMSIS device package (register/startup headers, e.g. `Include/stm32f446xx.h`). Carries **no** pin↔AF mux data; kept for future HAL/register work (codegen, Phase 3).

Both are read-only upstream data — never hand-edit them; corrections to pack data belong in the patch table in `nucleus-db/src/pack.rs` (the README flags pack-data inconsistency as the biggest time sink).

## Commands

`Makefile` is the task runner. `make` targets and CI run identical checks.

**Local workflow:**
- Full gate (run before pushing): `make check` (= `fmt-check` + `lint` + `test`)
- Build: `make build` / `cargo build -p <crate>`
- Test Rust: `make test` / `cargo test -p <crate>` / single: `cargo test -p <crate> <test_name>`
- Format / lint: `make fmt` · `make fmt-check` · `make lint` (`clippy -D warnings`)
- Extension: `cd extension && npm install && npm run build` (TypeScript, not part of `make check`)

**CLI surface (v1 + v2):**
- v1 complete: `nucleus init | check | build | flash | lsp | trace`
- v2 adds: `nucleus test` (run declarative/scripted tests on both backends), `nucleus history` (list versions + results), `nucleus show` (version details), `nucleus lockstep` (divergence detection)
- `nucleus check` exits non-zero on any conflict (CI-gatable)

**CI:** `.github/workflows/ci.yml` runs gate + cross-platform build on every PR; `release.yml` builds artifacts on `v*` tag.

## Github

**Github Workflow** 
- See information related to week1 and week2 at issue #19.
- If working on week1, see information on #20 then first post plan, and after completion post completion, push directly to branch `20-v2-week-1-verify-completion-prove-infrastructure`
- If working on week2, see information on #20 then first post plan, and after completion post completion  push directly to branch `21-v2-week-2-tests-ledger-lockstep-crown`
- Update comment 1, tick completed work on every push.
