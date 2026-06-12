# nucleus-cli

The `nucleus` binary — the CLI-first entry point for the toolchain.

## Status (Phase 2)

Implemented:

- `nucleus check [path]` — parse and validate an `stm32.toml` (default `./stm32.toml`) against the constraint database, print any conflicts, and exit non-zero on error so CI can gate on it. Exit `0` only when the config is conflict-free.

Declared but stubbed (each prints a "scheduled for Phase N" notice and exits non-zero) until their phases land:

- `nucleus init` / `build` / `flash` — Phase 3
- `nucleus lsp` — Phase 4
- `nucleus trace` — Phase 5

## Tests

`tests/cli.rs` drives the built binary against repo fixtures in `tests/fixtures/`, asserting the Phase-2 exit-code contract (clean → 0, PA5 collision → exactly one error and non-zero).
