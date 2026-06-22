# Scripted Tests

[Declarative tests](declarative-tests.md) cover one-line behaviors. M7 is the
escape hatch for everything else: a real Rust `#[test]` function, driving the
target over the same [`Backend`](hil-backends.md) trait, for assertions too
stateful or sequential to express as one string (a multi-step protocol
exchange, a GPIO-then-UART interaction, anything with branching logic).

## The device test-agent + RAM mailbox

Scripted tests don't poke arbitrary memory blind — they talk to a small,
fixed-protocol **test-agent** that runs on the target firmware itself, over a
RAM mailbox at a pinned address. The host SDK (`nucleus-test-sdk`) drives
that mailbox using only `Backend::read_mem32`/`write_mem32` — no new
backend capability needed, any `Backend` impl supports scripted tests for
free.

**Protocol (v1)**, mirrored byte-for-byte between `nucleus-test-sdk`'s Rust
constants and the agent firmware's C header:

- Mailbox base: `0x2000_0000` (start of SRAM, pinned by the agent's linker
  script).
- Magic: `0x4E54_4167` (`'NTAg'`), written by the agent **last**, after
  clocks/USART/GPIO init — so a matching magic always implies a fully
  initialized mailbox. The host polls for it rather than reading once, to
  avoid a boot race against backends that halt the target the instant the
  host attaches.
- Fields (offsets from base): `MAGIC`, `VERSION`, `SEQ`, `CMD`, `ARG0`,
  `ARG1`, `STATUS`, `RESP`.
- Status values: `IDLE` (0) → host sets `BUSY` (1) → agent posts `DONE` (2)
  or `ERR` (3).
- Commands: `PING`, `SET_GPIO`, `READ_GPIO`, `READ_REG`, `UART_TX`,
  `UART_RX_POLL` (returns `RX_NONE = 0xFFFF_FFFF` when nothing's buffered).

## The host SDK (`nucleus-test-sdk`)

```rust
use nucleus_test_sdk::AgentClient;

let mut client = AgentClient::new(backend); // backend: &mut dyn Backend
client.connect()?;                          // polls for magic + checks protocol version

client.set_gpio(nucleus_db::Port::A, 5, true)?;
let level = client.read_gpio(nucleus_db::Port::A, 5)?;

client.uart_tx(b'p')?;
let byte = client.uart_rx_poll()?;          // Option<u8>: None if nothing buffered

let reg = client.read_register(0x4002_0000)?;
```

Every method blocks until the agent posts `DONE`/`ERR` or a fixed poll
timeout (500 ms) elapses, surfacing as `SdkError::{BadMagic, VersionMismatch,
AgentError, Timeout, Hil}`.

## `[[test]]` schema for scripted tests

```toml
[[test]]
name = "uart_loopback"
type = "scripted"
script = "uart_loopback"   # the `cargo test` name nucleus-cli invokes
backend = "both"           # "qemu" | "hardware" | "both" (default)
```

`nucleus test` runs `cargo test <script> -- --exact --nocapture` for each
selected backend label, the same `--backend` filter and per-test `backend`
field intersection that declarative tests use (a `backend = "qemu"` test is
never force-run on hardware just because `--backend hardware` wasn't passed
— it's simply skipped).

## Worked example: UART loopback (both backends)

`crates/nucleus-hil/tests/fixtures/agent_loopback/stm32.toml`:

```toml
[device]
family = "STM32F411RE"

[peripherals.usart2]
tx = "PA2"
rx = "PA3"

[[test]]
name = "uart_loopback"
type = "scripted"
script = "uart_loopback"
backend = "both"
```

The test (`crates/nucleus-hil/tests/e2e_scripted_uart.rs`) starts each
backend, connects an `AgentClient`, sends bytes via `uart_tx`, and polls
`uart_rx_poll` until they come back. This is the test that empirically proved
QEMU's `netduinoplus2` machine fully models USART2 — even though the same
machine has no real GPIO model, so a `set_gpio`/`read_gpio` round-trip in a
scripted test only meaningfully verifies on the hardware backend.

## See also

- [Dual-Backend HIL Substrate](hil-backends.md) — the `Backend` trait this SDK is built on
- [Declarative Tests](declarative-tests.md)
- [Test History & CI](test-history.md)
