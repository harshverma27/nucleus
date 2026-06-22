# IRQ / NVIC Verifier

NVIC vectors and EXTI lines are shared, finite resources CubeMX never checks
for you: two GPIO pins wired to interrupts that happen to share an EXTI line,
a peripheral interrupt enabled with no vector modeled, or a DMA-vs-IRQ
priority inversion that quietly starves a completion handler. The M3 verifier
(`Conflict::IrqConflict`) catches all three at `nucleus check` time.

## The model (`nucleus-db/src/irq.rs`)

Pure data, hand-maintained (the pack XML carries no NVIC/IRQ information),
cross-checked by a reference-manual seed test:

- **EXTI → NVIC grouping**: lines 0–4 each get a dedicated vector
  (`EXTI0`..`EXTI4`); lines 5–9 share `EXTI9_5`; lines 10–15 share
  `EXTI15_10`. Identical on F446RE and F411RE (RM0390 / RM0383 Table 38).
- **Peripheral → vector map**: one row per modeled peripheral kind
  (USART/UART, SPI, I2C, TIM), restricted to the instances each family
  actually has. I2Cx is the one irregular case — **two** vectors per
  instance, `I2Cx_EV` (event) and `I2Cx_ER` (error); everything else has
  exactly one.

## The solver (`nucleus-compiler/src/irq.rs`)

Three independent checks, run in fixed order, each producing its own
`IrqConflict` entries:

1. **Unhandled IRQ** — a peripheral table sets `irq = true` for a peripheral
   the family's `IrqMap` has no vector for. *(Error.)*
2. **EXTI collision** — two or more `[[exti]]` entries resolve to the same
   EXTI line (shared across all eight GPIO ports — `PA0`, `PB0`, ... `PH0`
   all land on line 0). One conflict per contested line, naming every
   distinct port claiming it — not one conflict per colliding pair. *(Error.)*
3. **Priority inversion** — a peripheral with both `dma_priority` and
   `irq_priority` set, where the DMA priority is numerically *less* urgent
   (a larger number) than the IRQ priority: the peripheral's own ISR can
   then preempt the DMA-completion interrupt it's meant to hand off to.
   *(Warning — `nucleus check` does not fail on this alone.)*

`IrqConflict` is the first `Conflict` variant carrying its own explicit
`Severity` rather than always being an error — see [Pinout &
Peripheral Verification](pinout-verification.md) for the severity rule
`nucleus check`'s exit code follows.

## `stm32.toml` schema

```toml
[peripherals.usart2]
tx = "PA2"
rx = "PA3"
irq = true            # opt in to NVIC-vector verification (never inferred)
irq_priority = 5
dma = true
dma_priority = 2      # numerically smaller = more urgent

[[exti]]
pin = "PA0"
priority = 3
```

## Example: an EXTI line collision

```toml
# tests/fixtures/exti_collision.toml — PA0 and PB0 both resolve to EXTI0
[device]
family = "STM32F446RE"

[[exti]]
pin = "PA0"

[[exti]]
pin = "PB0"
```

```sh
$ nucleus check tests/fixtures/exti_collision.toml
tests/fixtures/exti_collision.toml: 1 conflict found:

  error: IRQ conflict [PA0]: EXTI0 is shared by PA0 and PB0 but only one can trigger it
```

## See also

- [Pinout & Peripheral Verification](pinout-verification.md)
- [Clock-Tree Solver](clock-tree.md)
- [DMA Arbitration](dma-arbitration.md)
- [Constraint Auto-Router](auto-router.md)
