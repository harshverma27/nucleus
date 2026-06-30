# Changelog

All notable changes to Nucleus are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Tagged releases (`v*`) are built and published automatically by
`.github/workflows/release.yml`, which also attaches GitHub-generated release
notes. This file is the curated, human-readable history.

## [Unreleased]

## [0.2.0] - 2026-06-30

This release is the complete **Nucleus v2 correctness loop**: static verification
is extended with three new conflict classes, a constraint auto-router replaces
manual pin assignment, and a dual-backend HIL substrate proves configurations on
real silicon and a simulated twin simultaneously. The loop closes with test
history, CI integration, and lockstep co-execution.

### Added
- **Clock-tree solver (M1).** Models the full STM32F4 PLL, prescaler, and
  per-peripheral frequency derivation chain. Rejects over-clocked buses and
  unreachable baud rates as `ClockConstraint` conflicts; `nucleus check` reports
  the derived frequency alongside each violation.
- **DMA arbitration solver (M2).** Models DMA1/DMA2 stream × channel × peripheral
  request maps for F446RE and F411RE. Detects stream collisions and suggests the
  nearest free alternative as part of the conflict message.
- **IRQ / NVIC verifier (M3).** Validates EXTI line ownership, preemption-priority
  ordering, and shared-pin peripheral IRQ priority. Introduces a `Severity` enum
  so warnings and errors are distinguished in `nucleus check` output and LSP
  diagnostics.
- **Constraint auto-router (M4) — `nucleus route`.** Backtracking router that
  assigns pins to peripherals from intent alone; emits `Unroutable` conflicts when
  no legal assignment exists. `nucleus route` writes the resolved pin assignments
  back into `stm32.toml` in place. LSP surfaces `Unroutable` with severity-aware
  diagnostics.
- **Dual-backend HIL substrate (M5).** `nucleus-hil` crate with a `Backend` trait
  implemented by both a QEMU/GDB-RSP backend and a hardware backend (OpenOCD
  telnet + SWO capture against a real NUCLEO). Both share the same observation and
  assertion API.
- **Declarative tests (M6) — `nucleus test`.** `[[test]]` blocks in `stm32.toml`
  declare assertions (`PinState`, `PinToggles`, `IrqFires`, `UartReceives`) that
  `nucleus test` runs against both backends. The compiler validates assertions at
  `nucleus check` time; the LSP adds hover and completion for assertion fields.
- **Scripted tests (M7).** `[[test]]` blocks with `type = "scripted"` compile and
  run a Rust test binary via `cargo test`, with the `nucleus-test-sdk` crate
  providing a mailbox `AgentClient` and a `Serial` helper (VCP + QEMU TCP) for
  firmware-side I/O. `nucleus test` selects backends by availability and runs
  scripted and declarative suites together.
- **Test history and CI-native HIL (M8/M9).** Every `nucleus test` run appends a
  `TestEntry` to `tests/test_history.json`. `nucleus history show` renders a
  colour-coded bar chart of recent pass/fail/skip counts in the terminal.
  A reusable `nucleus-history` crate exposes the schema for tooling. The existing
  `nucleus-action` CI composite now runs `nucleus test` and posts a pass/fail
  summary as a pull-request comment.
- **Lockstep co-execution (M10) — `nucleus lockstep`.** Runs the test suite on
  QEMU and hardware concurrently, collects `ObservationTrace` streams from both,
  and reports `DivergenceReport::Diverged` when the traces differ — catching
  simulator/silicon gaps that neither backend alone would surface.
- **PWM frequency / duty-resolution conflict detection.** The clock-tree solver
  now checks that the requested PWM frequency is achievable at the configured
  duty resolution on the target timer clock and emits a `ClockConstraint` conflict
  when it is not.
- **EXTI priority vs. shared-pin peripheral IRQ checking.** The NVIC verifier
  validates that an EXTI line's priority does not violate ordering constraints
  with the peripheral that shares the same GPIO pin.

## [0.1.0] - 2026-06-13

### Added
- **STM32F411RE support.** The NUCLEO-F411RE is a fully supported second board:
  `family = "STM32F411RE"` validates against a dedicated constraint database
  (generated from ST open pin data), `nucleus init --board NUCLEO-F411RE`
  scaffolds an F411-specific project, and `nucleus build` generates HAL code for
  it. A new `PeripheralUnavailable` conflict flags peripherals absent on the
  selected family, and the LSP resolves diagnostics/hover against the document's
  family. This fulfills the Phase 8 generality-proof criterion.
- **mdBook docs site + issue templates.** The `docs/` directory is now an
  mdBook source tree (Introduction, Installation, Quickstart, CLI Usage,
  Enabling ITM Trace, CI Integration), built and published to GitHub Pages on
  every push to `main` that touches `docs/`. The new "Enabling ITM Trace"
  chapter covers the firmware-side CoreSight register setup and the matching
  OpenOCD `tpiu`/`itm` commands. `.github/ISSUE_TEMPLATE/` adds structured bug
  report and feature request forms.

## [0.0.1] - 2026-06-13

### Added
- **Phase 7 — Distribution + Release Automation.** Crate metadata for
  publishing to crates.io (`cargo install nucleus-cli`); a release workflow that,
  on a `v*` tag, builds cross-platform CLI binaries (Linux/macOS/Windows,
  x86_64 + arm64) with checksums, drafts a GitHub Release with generated notes,
  and — when the registry/marketplace tokens are configured — publishes the
  crates and uploads the packaged `.vsix`. A reusable composite **nucleus-action**
  runs `nucleus check` + `nucleus build` and posts a PR summary; a copy-paste
  `nucleus.yml` is documented. Project docs, `CONTRIBUTING.md`, and dual
  MIT/Apache-2.0 license files.

## Released phases

These shipped on `main` ahead of the first tagged release:

- **Phase 6 — Trace Dashboard.** React/Canvas dashboard (log stream, live
  variable charts, CPU-load strip), resizable panels, dark mode, log export;
  DWT PC-sampling CPU-load decoding. Runs in the VS Code webview and standalone.
- **Phase 5 — ITM Decoder + Trace Backend.** `nucleus-itm` CoreSight decoder
  (never-panic, fuzz-tested) and `nucleus-trace` (translate + WebSocket on 7878);
  `nucleus trace`.
- **Phase 4 — LSP Server + Editor UX.** `nucleus-lsp` (diagnostics, hover,
  pin completion) and the VS Code LSP client.
- **Phase 3 — HAL Code Generation + Build.** Codegen (`nucleus_config.h` /
  `nucleus_init.c`) and `nucleus init`/`build`/`flash`.
- **Phase 2 — Config Parser + Constraint Solver.** `nucleus-compiler` and
  `nucleus check` (four conflict classes).
- **Phase 1 — Constraint Database Foundation.** `nucleus-db`, the deterministic
  F446RE pin/AF/peripheral table.

[Unreleased]: https://github.com/harshverma27/nucleus/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/harshverma27/nucleus/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/harshverma27/nucleus/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/harshverma27/nucleus/releases/tag/v0.0.1
