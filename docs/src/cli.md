# CLI usage

The `nucleus` binary is the whole toolchain. All commands are scriptable and
composable; `nucleus check` exits non-zero on conflicts so CI can gate on it.

```
nucleus <command>

  check   Validate stm32.toml against the constraint database
  init    Scaffold a new STM32 project
  build   Generate HAL init code and build firmware (.elf/.bin)
  flash   Flash the built firmware to a connected board
  trace   Decode ITM/SWO trace and stream events over a WebSocket
  lsp     Start the language server over stdio (used by the editor extension)
```

## `nucleus init [dir]`

Scaffolds a project: `stm32.toml`, `CMakeLists.txt`, a cross-toolchain CMake
file, `src/main.c`, a CI workflow, and `.gitignore`. Idempotent — it never
overwrites existing files.

```sh
mkdir blinky && cd blinky
nucleus init .
```

Use `--board` to target a specific NUCLEO board (default `NUCLEO-F446RE`):

```sh
nucleus init blinky --board NUCLEO-F411RE
```

Supported boards: `NUCLEO-F446RE`, `NUCLEO-F411RE`. The board sets the
`[device].family`, the linker script, the MCU define, the startup file, and the
default `clock_hz` in the scaffolded project.

## `nucleus check [path]`

Validates `stm32.toml` (default `./stm32.toml`) against the constraint database
for its `[device].family` (STM32F446RE or STM32F411RE) and prints any conflicts.
Detects pin collisions, AF mismatches, missing required pins, and disabled clock
domains. **Exit code:** `0` when clean, `1` on any conflict or read/parse error.

```sh
nucleus check
nucleus check configs/board-a.toml
```

## `nucleus build [dir]`

Validates the config, regenerates `src/generated/nucleus_config.h` +
`nucleus_init.c` (typed config structs + a single `Nucleus_Init()` that calls
stock ST HAL `Init` functions), then drives CMake + `arm-none-eabi-gcc` to
produce `firmware.elf` / `firmware.bin`.

> Requires `cmake` + `arm-none-eabi-gcc` and an STM32CubeF4 (HAL) checkout
> pointed at by `STM32CUBE_PATH`. Codegen still runs (and is observable) if the
> toolchain is missing; you'll get a clear error at the compile step. Build
> refuses to generate code for a conflicting config.

## `nucleus flash [dir]`

Programs `build/firmware.bin` to a connected board with `st-flash`.

## `nucleus trace`

Decodes the ITM/SWO byte stream and streams structured JSON events over a
WebSocket (default `:7878`) for the dashboard.

```sh
# Live, via OpenOCD's internal trace TCP port
nucleus trace --trace-tcp 127.0.0.1:3344 --openocd 127.0.0.1:4444

# Replay a captured raw-SWO file (great for development and demos)
nucleus trace --replay capture.swo

# Options
#   --config <stm32.toml>   read [[trace.variables]] (default stm32.toml)
#   --ws-port <port>        WebSocket port (default 7878)
```

Event shapes (JSON): `{"kind":"log","message":…}`,
`{"kind":"variable","port":…,"name":…,"type":…,"value":…}`,
`{"kind":"cpuload","load":…}`, `{"kind":"overflow"}`.

## `nucleus lsp`

Starts the language server over stdio. Normally spawned by the VS Code
extension, not run by hand.
