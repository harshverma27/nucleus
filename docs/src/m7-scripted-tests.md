# Scripted tests (M7)

M6 gave you **declarative** `[[test]]` blocks: a one-line `assertion` string
("pin PA5 is high within 10ms"), checked by the HIL runner without any code on
your side. M7 adds **scripted** tests for cases a one-liner can't express —
multi-step protocols, host-driven stimulus/response, anything that needs a
real Rust test function.

## When to use which

| | Declarative (M6) | Scripted (M7) |
|---|---|---|
| Defined by | `assertion = "..."` string | `script = "..."` cargo test name |
| Good for | single pin/register checks | multi-step protocols, sequencing, host-side stimulus |
| Runs via | the HIL runner's own assertion engine | `cargo test <script> -- --exact --nocapture` |
| Needs firmware build first | yes | no — the scripted test drives its own backend |

A config can mix both: `nucleus test` partitions `[[test]]` entries by `type`
and runs each kind through its own path. Scripted tests need no
`build/firmware` artifact — only declarative tests do.

```toml
[[test]]
name = "uart_loopback"
type = "scripted"
script = "uart_loopback"   # the #[test] fn name cargo will run
backend = "both"           # "qemu" | "hardware" | "both" (default: both)
```

`backend` on a scripted test is a *filter*, not an override: `nucleus test
--backend hardware` will skip a `backend = "qemu"` test rather than force it
onto hardware, same skip semantics as the declarative path.

## The device test-agent: a RAM mailbox

For scripted tests that need to drive the device interactively (set a GPIO,
read a register, send/receive UART bytes) without designing a new firmware
protocol per test, Nucleus ships a generic **device test-agent** — a small
firmware blob that idles in a loop, watching one fixed memory location for
commands, and a host-side SDK (`nucleus_test_sdk`) that talks to it over
whatever the backend exposes (QEMU's GDB-stub memory access, or SWD on real
hardware).

> The protocol's single source of truth is
> `crates/nucleus-test-sdk/src/protocol.rs`. The firmware agent
> (`crates/nucleus-hil/tests/fixtures/agent_loopback/agent.c`) mirrors it
> byte-for-byte. If you change one, change the other.

### Layout

The mailbox is 8 little-endian `u32` fields, starting at a fixed SRAM address:

| Field | Offset | Meaning |
|---|---|---|
| `magic` | `0x00` | `0x4E54_4167` (`'NTAg'`), written last during boot so a nonzero read means "fully initialized" |
| `version` | `0x04` | protocol version (see below) |
| `seq` | `0x08` | sequence counter the host bumps before each command; reserved for future de-duplication. Synchronization does **not** depend on it — the agent acts only on `status == BUSY`, which the host writes *last* (after the args/cmd), so a command is never observed half-written. |
| `cmd` | `0x0C` | command id (see table) |
| `arg0` | `0x10` | command argument 0 |
| `arg1` | `0x14` | command argument 1 |
| `status` | `0x18` | status machine (see below) |
| `resp` | `0x1C` | command result, valid once `status == DONE` |

Base address: `0x2000_0000` — the start of SRAM, pinned by the agent's linker
script. The host needs no ELF symbol lookup to find it.

### Status machine

```
IDLE (0) --host writes cmd/args, then STATUS=BUSY--> BUSY (1)
BUSY (1) --agent finishes, writes resp, then STATUS=DONE--> DONE (2)
BUSY (1) --agent hits an error--> ERR (3)
```

The host polls `status` after issuing a command and reads `resp` once it sees
`DONE` (or returns an `AgentError` on `ERR`, or a `Timeout` if neither shows up
in time).

### Commands

| Name | id | `arg0` | `arg1` | `resp` |
|---|---|---|---|---|
| `PING` | 0 | — | — | protocol version |
| `SET_GPIO` | 1 | encoded pin (see below) | level (0/1) | — |
| `READ_GPIO` | 2 | encoded pin | — | level (0/1) |
| `READ_REG` | 3 | register address | — | register value |
| `UART_TX` | 4 | byte to send | — | — |
| `UART_RX_POLL` | 5 | — | — | byte, or `0xFFFF_FFFF` (`RX_NONE`) if nothing arrived |

### Port/pin encoding

`encode_pin(port, pin)` packs a port + pin number into one `u32` argument:

```
arg = (port_index << 8) | pin
```

Port index follows `nucleus_db::Port`'s declaration order: **A=0, B=1, C=2,
D=3, E=4, F=5, G=6, H=7**.

## Protocol versioning

`version` is checked by `AgentClient::connect()`: if the firmware's `version`
doesn't match the SDK's `PROTO_VERSION` constant, `connect()` returns
`SdkError::VersionMismatch` rather than silently misinterpreting fields. This
lets firmware and host SDK evolve independently — bump `PROTO_VERSION` (and
the firmware's matching `#define`) whenever you make an incompatible layout
or command-table change, and old firmware paired with a new SDK (or vice
versa) fails loudly at `connect()` instead of corrupting a test result.

## Using `nucleus_test_sdk`

```rust
use nucleus_test_sdk::{AgentClient, Serial};

// `backend` is anything implementing `nucleus_hil::backend::Backend`
// (QemuBackend or HardwareBackend), already started.
//
// Open the UART side-channel first — `serial_port()` borrows `&backend`,
// so it must happen before the `&mut backend` the AgentClient holds. On
// QEMU the port is allocated dynamically and reported by the backend; on
// hardware it is the ST-Link Virtual COM Port device path.
let mut serial = Serial::open_tcp(&format!("127.0.0.1:{}", backend.serial_port().unwrap()))
    .expect("open QEMU USART socket"); // hardware: Serial::open_device("/dev/ttyACM0", 115200)

let mut client = AgentClient::new(&mut backend);
client.connect().expect("agent handshake");

// TX: ask the agent to send a byte out its USART, observe it on the wire.
client.uart_tx(0x5A).expect("uart_tx");
let byte = serial
    .read_byte(std::time::Duration::from_millis(200))
    .expect("serial read")
    .expect("byte arrived");
assert_eq!(byte, 0x5A);

// RX: write a byte on the wire, ask the agent if it arrived.
serial.write_byte(0x3C).expect("serial write");
let received = client.uart_rx_poll().expect("uart_rx_poll");
assert_eq!(received, Some(0x3C));
```

See `crates/nucleus-hil/tests/e2e_scripted_uart.rs` for the full two-backend
version of this, including skip-on-missing-tooling behavior.

## The GPIO-on-QEMU gap

QEMU's `netduinoplus2` machine (used for the F4 HIL backend) has no GPIO
peripheral model: writes to `BSRR` are dropped and `IDR` always reads back 0.
So `SET_GPIO`/`READ_GPIO` mailbox commands only prove anything on **real
hardware** — on QEMU they execute without error but don't reflect real pin
state. UART and the mailbox handshake itself (`PING`, `connect()`) are fully
functional on both backends, because QEMU's `netduinoplus2` does model USART2
and route it to a serial socket. Scripted tests that need GPIO assertions
should either gate on `NUCLEUS_TEST_BACKEND=hardware` or skip the GPIO half on
QEMU, mirroring `e2e_scripted_uart.rs`'s `assert_gpio` (hardware-only) vs.
`assert_loopback` (both backends) split.
