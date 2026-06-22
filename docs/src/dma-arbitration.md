# DMA Arbitration

STM32F4's DMA1/DMA2 controllers have eight streams each, and each stream
multiplexes eight peripheral-request channels. Two peripherals can be wired
in CubeMX onto the same physical stream — both will compile and flash, and
the second one's DMA transfers will simply never fire. Nucleus catches this
at `nucleus check` time instead of at "why is my second sensor not updating".

## The model (`nucleus-db/src/dma.rs`)

A hand-maintained, family-parameterized map of:

- DMA1 and DMA2, their eight streams each.
- Each stream's eight request channels.
- The peripheral-request table: which `(peripheral, direction)` — e.g.
  `(usart2, Rx)` — is servable by which `(controller, stream, channel)` slots.

Cited to ST's reference manuals: RM0390 Tables 28–29 (F446RE), RM0383 Tables
27–28 (F411RE). Like the clock-tree model, this can't be generated from pack
XML (no DMA data there) so it's hand-maintained and seed-tested.

## The solver (`nucleus-compiler/src/dma.rs`)

`validate()` is a deterministic, greedy first-fit:

1. Collect every `dma = [...]` request across configured peripherals, in
   `BTreeMap` peripheral order.
2. For each request, in order, assign the **first candidate stream that's
   still free**.
3. If every candidate for a request is already taken, record a
   `Conflict::DmaCollision` against that stream — once per contested stream,
   not once per colliding pair — naming the existing holder, the new
   contender, and (if either side has a free alternative slot) a suggestion.

## `stm32.toml` schema

```toml
[peripherals.i2c1]
sda = "PB7"
scl = "PB6"
dma = ["rx"]        # request DMA for specific signals…

[peripherals.uart5]
tx = "PC12"
rx = "PD2"
dma = true          # …or `true` for every direction the peripheral supports
```

## Example: a stream collision

```toml
# tests/fixtures/dma_collision.toml — I2C1_RX and UART5_RX both map to
# DMA1 stream 0; I2C1_RX has a free alternative on stream 5, UART5_RX doesn't.
[device]
family = "STM32F446RE"

[peripherals.i2c1]
sda = "PB7"
scl = "PB6"
dma = ["rx"]

[peripherals.uart5]
tx = "PC12"
rx = "PD2"
dma = ["rx"]
```

```sh
$ nucleus check tests/fixtures/dma_collision.toml
tests/fixtures/dma_collision.toml: 1 conflict found:

  error: DMA collision: I2C1 and UART5 both need DMA1 stream 0 (move I2C1 to DMA1 stream 5 (channel 1))
```

## See also

- [Pinout & Peripheral Verification](pinout-verification.md)
- [Clock-Tree Solver](clock-tree.md)
- [IRQ / NVIC Verifier](irq-verifier.md)
