# nucleus-cli

The `nucleus` binary — the CLI-first entry point for the toolchain.

## Status (Phases 2–3)

Implemented:

- `nucleus check [path]` — parse and validate an `stm32.toml` (default `./stm32.toml`) against the constraint database, print any conflicts, and exit non-zero on error so CI can gate on it.
- `nucleus init [dir]` — scaffold a new project (`stm32.toml`, `CMakeLists.txt`, `cmake/` cross-toolchain file, `src/main.c`, `.github/workflows/ci.yml`, `.gitignore`). Idempotent: never overwrites existing files.
- `nucleus build [dir]` — validate the config, generate `src/generated/nucleus_config.h` + `nucleus_init.c`, then drive CMake + arm-none-eabi-gcc to produce `firmware.elf`/`.bin`. Refuses to generate code for a conflicting config; a missing toolchain yields a clear error (codegen still runs).
- `nucleus flash [dir]` — program `build/firmware.bin` to the board with `st-flash`.

Declared but stubbed (print a "scheduled for Phase N" notice and exit non-zero):

- `nucleus lsp` — Phase 4
- `nucleus trace` — Phase 5

> Building actual firmware requires the ARM cross toolchain and an STM32CubeF4 (HAL) checkout, pointed at via `STM32CUBE_PATH` in the scaffolded `CMakeLists.txt`.

## Tests

`tests/cli.rs` drives the built binary against repo fixtures and freshly scaffolded temp projects: the Phase-2 exit-code contract (clean → 0, PA5 collision → exactly one error), `init` scaffolding + idempotence, that a scaffolded config passes `check`, and that `build` codegen emits valid HAL sources (and refuses a conflicting config).
