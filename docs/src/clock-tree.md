# Clock-Tree Solver

CubeMX's clock configurator lets you build a clock tree visually and never
checks whether the numbers it produces are actually legal silicon — an APB1
bus quietly clocked at 180 MHz instead of its 45 MHz limit will boot and run
until something subtly misbehaves. Nucleus's clock-tree solver (`Conflict::ClockConstraint`)
rejects that config before it ever reaches a board.

## The model (`nucleus-db/src/clock.rs`)

A hand-maintained, family-parameterized description of:

- Oscillator sources: `Hsi`, `Hse`, `Lsi`, `Lse`.
- The main PLL's M/N/P/Q divider ranges and VCO input/output frequency ranges.
- The SYSCLK source set and the AHB/APB1/APB2 prescaler sets.
- Per-family silicon limits, cited to ST's reference manuals (RM0390 for
  F446RE, RM0383 for F411RE):

  | Bus | F446RE limit |
  |---|---|
  | SYSCLK | ≤ 180 MHz |
  | AHB | ≤ 180 MHz |
  | APB1 | ≤ 45 MHz |
  | APB2 | ≤ 90 MHz |

This module is **pure data** — it performs no arithmetic. It can't be
generated from ST's pack XML (that data carries pin↔AF mappings only, no
clock-tree information), so it's hand-maintained and cross-checked by a seed
test against the reference manuals.

## The solver (`nucleus-compiler/src/clocks.rs`)

`validate()` walks the effective config in four stages, stopping early at the
first stage that fails (a downstream frequency number computed from an
already-illegal divider would just produce a confusing cascade of unrelated
errors):

1. **PLL divider legality** — M/N/P/Q each checked against the model's ranges.
2. **Prescaler legality** — AHB/APB1/APB2 prescaler must be one of the
   hardware's allowed divisor values.
3. **PLL VCO range** — input and output VCO frequency must land inside the
   silicon's specified band.
4. **Silicon limits** — SYSCLK, AHB, APB1, APB2 each checked against the
   family's max, in that fixed order.

## `stm32.toml` schema

```toml
[clocks]
source         = "pll"   # "hsi" | "hse" | "pll" (default "pll")
pll_source     = "hse"   # PLL input: "hsi" | "hse"
pll_m          = 8
pll_n          = 360
pll_p          = 2
pll_q          = 7       # optional, only needed if something derives from PLLQ
apb1_prescaler = 2
apb2_prescaler = 1
ahb1 = true               # bus enable flags; default true if [clocks] section omitted
apb1 = true
apb2 = true
```

Omitting `[clocks]` entirely is valid and enables every bus by default.

## Example: an over-clocked APB1

```toml
# tests/fixtures/overclock_apb1.toml — 180 MHz HCLK ÷ prescaler 1 = 180 MHz
# on a bus whose limit is 45 MHz. CubeMX accepts this; Nucleus doesn't.
[device]
family = "STM32F446RE"

[clocks]
source = "pll"
pll_source = "hse"
pll_m = 8
pll_n = 360
pll_p = 2
apb1_prescaler = 1

[peripherals.usart2]
tx = "PA2"
rx = "PA3"
```

```sh
$ nucleus check tests/fixtures/overclock_apb1.toml
tests/fixtures/overclock_apb1.toml: 1 conflict found:

  error: clock constraint [APB1]: APB1 = 180 MHz exceeds the 45 MHz limit
```

Other reasons you'll see inside `clock constraint [<node>]: ...`: `PLL M = 3
is outside the valid 2–63 range`, `PLL VCO output = 432 MHz is outside the
100–432 MHz range`, `APB2 prescaler 3 is not one of the allowed values {1, 2,
4, 8, 16}`.

## Baud-rate reachability

Peripheral baud/bit rates (USART, SPI, I2C) are derived from whichever bus
clocks them (`nucleus-db::peripheral_bus`). If the resolved bus frequency
can't reach a requested baud rate within the peripheral's divider range, that
surfaces as the same `ClockConstraint` conflict, on the peripheral's node
rather than `SYSCLK`/`AHB`/`APBn`.

## See also

- [Pinout & Peripheral Verification](pinout-verification.md) — the base solver pass
- [DMA Arbitration](dma-arbitration.md), [IRQ / NVIC Verifier](irq-verifier.md) — the other M1–M3 conflict classes
