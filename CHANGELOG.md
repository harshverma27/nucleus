# Changelog

All notable changes to Nucleus are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Tagged releases (`v*`) are built and published automatically by
`.github/workflows/release.yml`, which also attaches GitHub-generated release
notes. This file is the curated, human-readable history.

## [Unreleased]

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

[Unreleased]: https://github.com/harshverma27/nucleus/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/harshverma27/nucleus/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/harshverma27/nucleus/releases/tag/v0.0.1
