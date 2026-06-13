# Introduction

A CLI-first STM32 developer platform: declarative `stm32.toml` → validated HAL
init code → flashed firmware, plus a real-time ITM trace dashboard.

Nucleus replaces STM32CubeIDE/CubeMX's graphical pin configuration and
proprietary debug tooling with a version-controllable, CI-friendly CLI and a
thin VS Code extension.

**[Watch the demo video](https://youtu.be/8zDZzE12wec)**

## At a glance

```toml
# stm32.toml
[device]
family   = "STM32F446RE"
board    = "NUCLEO-F446RE"
clock_hz = 180_000_000

[peripherals.usart2]
tx   = "PA2"
rx   = "PA3"
baud = 115200
```

```sh
nucleus check     # validate against the constraint database
nucleus build     # generate HAL init code + build firmware
nucleus trace     # decode ITM/SWO and stream to the dashboard
```

Nucleus supports two NUCLEO boards out of the box: **NUCLEO-F446RE**
(`STM32F446RE`) and **NUCLEO-F411RE** (`STM32F411RE`) — pick one with
`nucleus init --board <name>`.

## What's in this book

- **[Installation](installation.md)** — install the `nucleus` CLI and the VS
  Code extension.
- **[Quickstart: Blink an LED](quickstart.md)** — from a clean machine to a
  blinking LED on a NUCLEO-F446RE.
- **[CLI Usage](cli.md)** — `check`, `init`, `build`, `flash`, `lsp`, `trace`.
- **[Enabling ITM Trace](itm-trace.md)** — wire up SWO/ITM in firmware and
  OpenOCD for `nucleus trace`.
- **[CI Integration](ci.md)** — gate PRs with `nucleus check` via the reusable
  action.
