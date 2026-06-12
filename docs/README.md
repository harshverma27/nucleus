# Nucleus documentation

A CLI-first STM32 developer platform: declarative `stm32.toml` → validated HAL
init code → flashed firmware, plus a real-time ITM trace dashboard.

- **[Installation](installation.md)** — install the `nucleus` CLI and the VS Code extension.
- **[CLI usage](cli.md)** — `check`, `init`, `build`, `flash`, `lsp`, `trace`.
- **[CI integration](ci.md)** — gate PRs with `nucleus check` via the reusable action.

> A full mdBook documentation site (published to GitHub Pages) and the firmware
> integration guide land in Phase 8. These pages are the canonical reference
> until then.

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

Nucleus targets the **NUCLEO-F446RE** through Phase 7; a second MCU family
(STM32L476RG) lands in Phase 8 to prove the design generalizes.
