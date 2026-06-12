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

## Status (Phase 3 — complete)

- `codegen` — lowers a validated config to two C files via `generate(&config, &db)`:
  - `nucleus_config.h` — a typed config struct per peripheral, `extern` HAL handle declarations, and the `Nucleus_Init()` prototype.
  - `nucleus_init.c` — resolved config struct instances, handle definitions, and a single `Nucleus_Init()` that enables GPIO/peripheral clocks, configures alternate-function muxing (`GPIO_AF<n>_<PERIPH>` numbers from `nucleus-db`), and calls stock `HAL_*_Init` functions.

The generated code never reimplements the HAL — only `Init` calls with resolved parameters, so HAL point-releases don't break output. Output is byte-deterministic (tested). Tested HAL family: STM32F4 (`stm32f4xx_hal.h`).
