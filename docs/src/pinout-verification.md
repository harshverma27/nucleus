# Pinout & Peripheral Verification

The base layer every other feature in this book builds on: a deterministic
pin/alternate-function/peripheral database for the F446RE and F411RE, and a
solver that checks a declared `stm32.toml` against it.

## The database (`nucleus-db`)

`nucleus-db` is generated at **build time** from ST's own
[`STM32_open_pin_data`](https://github.com/STMicroelectronics/STM32_open_pin_data)
pack XML (vendored under `crates/nucleus-db/packdata/`), not hand-written. For
each supported family it produces:

- Every physical pin and its package position.
- Every alternate-function (AF) mapping: which pin, on which AF number, routes
  to which peripheral signal (e.g. `PA9` AF7 → `USART1_TX`).
- The peripheral instance list for that family (e.g. F411RE has no
  USART3/UART4/UART5; F446RE does).

Because it's generated deterministically from the same pack data every build,
the database never drifts between machines — there's no "works on my CubeMX
install" class of bug. Pack-data inconsistencies are the one place hand
correction is allowed: small patches live in `nucleus-db/src/pack.rs`'s patch
table, never by editing the vendored XML.

## What `nucleus check` validates

`nucleus check [path]` (default `./stm32.toml`) parses your config and runs it
through the solver in `nucleus-compiler/src/solver.rs`, which reports a
`Conflict` for each of:

| Conflict | Trigger |
|---|---|
| `PinCollision` | Two peripheral signals assigned to the same physical pin. |
| `AfMismatch` | A pin assigned to a peripheral signal that pin doesn't actually expose (no AF row for it on this family). |
| `InvalidPin` | A pin role's string value isn't a real pin name (typo). |
| `MissingPin` | A peripheral declared without a required pin role set. |
| `ClockDomainDisabled` | A peripheral configured while its bus (`[clocks]`) is turned off. |
| `PeripheralUnavailable` | An instance that doesn't exist on the selected family (e.g. `usart5` on an F411RE). |

(Clock-tree, DMA, IRQ, and auto-routing conflicts build on this same `Conflict`
enum and solver pass — see their own pages.)

Exit code: `0` clean, `1` on any conflict or parse error — designed to gate CI
(see [CI Integration](ci.md)).

### Example: a pin collision

```toml
# tests/fixtures/pa5_collision.toml — PA5 claimed by both SPI1_SCK and TIM2_CH1
[device]
family = "STM32F446RE"

[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"

[peripherals.tim2]
channel1 = "PA5"
```

```sh
$ nucleus check tests/fixtures/pa5_collision.toml
tests/fixtures/pa5_collision.toml: 1 conflict found:

  error: pin collision on PA5: assigned to SPI1_SCK and TIM2_CH1
```

### Example: clean

```toml
# tests/fixtures/clean.toml
[device]
family = "STM32F446RE"
board = "NUCLEO-F446RE"
clock_hz = 180_000_000

[peripherals.usart2]
tx = "PA2"
rx = "PA3"
baud = 115200

[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
nss = "PA4"
mode = 0

[peripherals.i2c1]
sda = "PB9"
scl = "PB8"
speed = "standard"

[peripherals.tim2]
channel1 = "PA0"
channel2 = "PA1"
frequency_hz = 1000
```

```sh
$ nucleus check tests/fixtures/clean.toml
tests/fixtures/clean.toml OK — 4 peripheral(s), no conflicts.
```

## Where this surfaces in the editor

`nucleus-lsp` (`analysis.rs`) maps every `Conflict` to the most relevant byte
span in the source `stm32.toml` and reports it as an LSP diagnostic, so VS
Code (via the extension's LSP client) underlines the exact offending line —
no separate "run check" step needed while editing.

## See also

- [Clock-Tree Solver](clock-tree.md) — frequency-domain checks (M1)
- [DMA Arbitration](dma-arbitration.md) — stream/channel contention (M2)
- [IRQ / NVIC Verifier](irq-verifier.md) — interrupt coverage (M3)
- [Constraint Auto-Router](auto-router.md) — fill in pins automatically (M4)
