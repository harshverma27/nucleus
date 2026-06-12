# nucleus-compiler

The Nucleus pinmux compiler: the `stm32.toml` → diagnostics pipeline.

## Status (Phase 2 — complete)

- `config` — parses `stm32.toml` into typed structs (`serde` + `toml`). Peripheral tables are kept as raw key→value maps so the parser stays stable as peripherals are added.
- `model` — hand-maintained tables mapping config keys (`tx`, `mosi`, `sda`, `channel1`…) to database signal names, marking required vs. optional pins, and assigning each peripheral to an F446 bus (APB1/APB2/AHB1).
- `solver` — validates a parsed config against [`nucleus-db`] and returns the four Phase-2 conflict classes:
  1. **pin collision** — two signals on one physical pin
  2. **AF mismatch** — a pin that doesn't expose the requested signal on this MCU
  3. **missing required pin** — a peripheral declared without a required pin
  4. **clock domain disabled** — a peripheral whose bus clock is turned off via `[clocks]`
- `check` / `check_family` — one-call entry points used by `nucleus-cli` (and, later, the LSP).

Conflicts are returned in deterministic order; a doubly-used pin yields exactly one collision error.

## Not yet

HAL code generation (`nucleus_config.h` / `nucleus_init.c`) lands in Phase 3.
