# Changelog

All notable changes to Nucleus are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Tagged releases (`v*`) are built and published automatically by
`.github/workflows/release.yml`, which also attaches GitHub-generated release
notes. This file is the curated, human-readable history.

## [Unreleased]

## [0.2.0] - 2026-06-30

### Fixed
- **Divide-by-zero panic in clock-tree solver.** `default_effective()` divided by
  `vco_in` without guarding against zero; a family model with no HSE oscillator
  panicked at runtime instead of surfacing a solver diagnostic. Now returns
  `Result` and maps `vco_in == 0` to `ResolveError::ZeroDivider`.
- **`PinToggles` assertion always tested the wrong pin.** The parsed `Pin` value
  from `parse_pin_or_fail()` was silently discarded; `backend.sample()` queried a
  hardcoded observable (TIM2 counter / PA5) regardless of what the test specified.
  Now polls `backend.pin(port, pin_num)` directly on the asserted pin.
- **Lockstep `collect()` folded backend errors into false divergence reports.**
  `Err(_) => break` treated a dropped GDB connection the same as a clean
  end-of-stream, producing a truncated trace that `compare()` reported as
  `Diverged`. `collect()` now returns `Result` and the caller surfaces the error.
- **QEMU `read_mem32`/`write_mem32` left the target permanently halted on a
  transient resume error.** `continue_execution().await?` propagated via `?` after
  a successful read/write, discarding the result and halting the target for the
  rest of the run. Resume now retries once via `resume_with_retry()` and its
  outcome is tracked separately from the read/write result.
- **Hardware `read_memory` left the STM32 target permanently halted on a read
  timeout.** An early `?` on `read_until_value` skipped the `resume` telnet
  command, causing OpenOCD to keep the target halted. Read and resume results are
  now tracked independently; resume is always attempted once halt succeeds.
- **Hardware `write_memory` discarded a successful write on resume failure.**
  The halt/mww/resume sequence was chained behind a single `?`; a resume error
  after a successful write propagated as the function's return value. Write and
  resume results are now tracked separately with the same pattern as `read_memory`.
- **PWM frequency constraint check silently passed invalid configs on u64
  overflow.** `(freq as u64) * arr_plus_one` wrapped silently for large TOML
  `i64` frequency values, causing the `divisor > timer_clk` guard to evaluate
  false. Replaced with `checked_mul`; an overflowing product is treated as an
  unreachable frequency and emits a conflict.
- **`Serial::Port` `read_byte` treated `WouldBlock` as an error instead of a
  timeout.** Some serialport backends return `WouldBlock` on timeout rather than
  `TimedOut`; the `Port` branch only caught `TimedOut`, breaking the `Ok(None)`
  timeout contract. Now catches both, consistent with the `Tcp` branch.
- **`connect()` returned `BadMagic(0)` when the device never booted.** A target
  that never published the mailbox magic word (no probe attached, power issue)
  returned `BadMagic(0)` — indistinguishable from wrong firmware. The deadline
  check is now split: magic still zero at timeout → `SdkError::Timeout`; non-zero
  mismatch → `BadMagic` as before.
- **`nucleus lockstep` suppressed `UnknownFamily` warnings.** `run_lockstep` called
  `nucleus_compiler::check()` which silently fell back to the F446RE database for
  unknown families. Now uses `check_family()`, consistent with `run_check` and
  `run_test`.

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
