# M7 — Scripted Tests: Design

**Date:** 2026-06-21
**Milestone:** v2 M7 (issue #21)
**Status:** approved, pre-implementation

## Goal

Add the power-user escape hatch for tests that need host-driven stimulus and
multi-step sequences beyond M6's passive declarative assertions: a minimal,
optional device **test-agent** (firmware, never ships in production) plus a Rust
**host SDK** (`nucleus_test_sdk`) for stimulus/response tests. Deliver a UART
loopback fixture running on both backends, integrate with the M6 `[[test]]`
schema via `type = "scripted"`, and document the protocol + SDK.

## Background / problem

M6's backends are observe-only. The `Backend` trait exposes `pin`, `register`,
`await_itm_event`, `sample` — no host→device path. M6's `UartEcho` assertion is
in fact passive: it only waits for an ITM event and assumes the firmware
self-loops. There is no way for the host to drive an input and observe the
resulting output. M7 fills exactly that gap.

## Locked design decisions

1. **Transport = RAM mailbox.** A fixed RAM struct the host reads/writes via the
   memory access each backend already has (OpenOCD telnet `mdw`/`mww` on
   hardware; gdbstub `m`/`M` on QEMU). One mechanism, both backends, no new wire
   protocol. Chosen over SEGGER RTT (QEMU has no native RTT host; ring buffers
   add framing) and ITM stimulus port (device→host only, cannot carry commands).
2. **Scope = full vertical + live-hardware verification this session.** All five
   acceptance items, green on QEMU in CI, and the loopback proven on a real
   NUCLEO-F411RE this session (board connected). Hardware leg degrades to SKIP
   when no board is present (matches M5/M6).
3. **Loopback = two-channel via VCP.** Host drives the device over the SWD
   mailbox AND reads/writes the UART itself over the ST-Link Virtual COM Port
   (`/dev/ttyACM*` on hardware, QEMU `-serial` TCP on emulator). No jumper.
4. **Scripted execution = shell out to `cargo test`.** A `[[test]]
   type = "scripted"` entry names a cargo test; `nucleus test` runs
   `cargo test -p nucleus-hil --test <file> <name>` and parses pass/fail.

## Architecture

### 1. Device test-agent (firmware)

New fixture `crates/nucleus-hil/tests/fixtures/agent_loopback/` for F411RE, C +
startup.s + linker script, mirroring the existing `blink_itm` fixture layout.
Everything agent-related is behind `#ifdef NUCLEUS_TEST_MODE`.

- **Mailbox** struct pinned at a fixed RAM address via a dedicated linker
  section `.nucleus_agent` placed at the start of SRAM (e.g. `0x2000_0000`), so
  the host uses a compile-time constant and needs no ELF symbol parsing. Layout
  (all `u32`, little-endian):

  ```
  offset  field     notes
  0x00    magic     'NTAg' = 0x4E544167
  0x04    version   protocol version, = 1
  0x08    seq       host bumps on each new command
  0x0C    cmd       command id (see below)
  0x10    arg0
  0x14    arg1
  0x18    status    0=IDLE 1=BUSY 2=DONE 3=ERR
  0x1C    resp      command result
  ```

- **Handshake:** host writes `arg0/arg1/cmd`, increments `seq`, sets
  `status=BUSY`. The agent main loop polls `status`; on `BUSY` it executes `cmd`,
  writes `resp`, then sets `status=DONE` (or `ERR` on bad command/version). Host
  polls `status` until `DONE`/`ERR` or a bounded timeout.

- **Commands (protocol v1):**
  | id | name          | arg0          | arg1  | resp                |
  |----|---------------|---------------|-------|---------------------|
  | 0  | PING          | —             | —     | version             |
  | 1  | SET_GPIO      | port+pin enc  | level | 0                   |
  | 2  | READ_GPIO     | port+pin enc  | —     | level (0/1)         |
  | 3  | READ_REG      | abs addr      | —     | word at addr        |
  | 4  | UART_TX       | byte          | —     | 0                   |
  | 5  | UART_RX_POLL  | —             | —     | 0xFFFFFFFF if none, else byte |

  `port+pin enc` = `(port_index << 8) | pin_num`.

- USART2 configured PA2 TX / PA3 RX, 115200 8N1, routed to the ST-Link VCP on
  hardware and QEMU's `-serial` chardev on the emulator. The agent emits an ITM
  `agent_ready` marker on boot so existing M5/M6 observation paths still work.

### 2. Backend trait extension (`crates/nucleus-hil/src/backend.rs`)

Add two memory primitives to the `Backend` trait:

- `fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError>`
- `fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError>`

Implementations:
- **Hardware:** `read_mem32` = telnet `mdw <addr>` (the existing register-read
  path is refactored onto this); `write_mem32` = telnet `mww <addr> <value>`
  (new), reusing the same OpenOCD console connection with `halt`/op/`resume`.
- **QEMU:** `read_mem32` = gdbstub `m<addr>,4`; `write_mem32` = gdbstub
  `M<addr>,4:<bytes>` (new).

Existing `register(peripheral, offset)` is re-expressed over `read_mem32` so
there is one memory-read path per backend. Backends stay protocol-dumb; all
mailbox logic lives in the SDK.

### 3. Host SDK — new crate `crates/nucleus-test-sdk/` (`nucleus_test_sdk`)

- `AgentClient<'a>` wraps `&'a mut dyn Backend` + the mailbox base address.
  Methods: `ping() -> u32`, `set_gpio(port, pin, level)`, `read_gpio(port, pin)
  -> bool`, `read_register(addr) -> u32`, `uart_tx(byte)`, `uart_rx_poll() ->
  Option<u8>`. Each performs write-args → bump-seq → set-BUSY → poll-status
  (bounded timeout) → read-resp. On connect, verifies `magic` and `version`;
  a mismatch returns a typed error so firmware and SDK can evolve independently.
- `Serial` helper: QEMU = `std::net::TcpStream` to the emulator's `-serial`
  TCP port; hardware = `/dev/ttyACM*` opened in raw mode. This pulls one
  dependency (`serialport`), scoped to the SDK crate only (the zero-dep rule is
  for `nucleus-itm`, not here). Methods: `write_byte`, `read_byte(timeout)`.
- `assert_responds_within(dur, f)` timing helper.

Dependency direction: `nucleus-test-sdk` depends on `nucleus-hil` (for
`Backend` and friends). It does not depend back on the CLI.

### 4. M6 integration (compiler + `nucleus test`)

- **Schema:** `[[test]]` gains optional `type` (`"declarative"`, default |
  `"scripted"`) and `script` (string: cargo test name). Parsed via serde
  alongside the existing M6 fields.
- **Validator:** `type = "scripted"` requires `script` and forbids `assertion`;
  `type = "declarative"` (or omitted) requires `assertion` as today. Bad
  combinations produce a `Conflict::InvalidTest` diagnostic (the existing
  variant).
- **Compiler:** `CompiledTest` carries either the existing declarative assertion
  or a new scripted variant `{ script: String }`. `BackendSelect` still applies.
- **`nucleus test`:** for a scripted entry, runs
  `cargo test -p nucleus-hil --test <fixture-test-file> <script> -- --exact`,
  passing the selected backend through `NUCLEUS_TEST_BACKEND` env var, and maps
  the cargo exit/output to pass/fail. Declarative entries run unchanged via the
  M6 assertion runner.

### 5. Fixture, e2e, docs

- `crates/nucleus-hil/tests/e2e_scripted_uart.rs`: an SDK-driven test.
  - **TX path:** host issues `UART_TX(b)` over the mailbox → device transmits on
    USART2 → host reads `b` back from the VCP.
  - **RX path:** host writes `b` to the VCP → device USART2 RX receives → host
    issues `UART_RX_POLL` over the mailbox and reads `b`.
  - Asserts each round trip completes within 10 ms.
  - Runs on QEMU always; the hardware leg is gated/skipped like
    `e2e_hardware_replay.rs` when no board is present.
- Docs page under `docs/src/`: agent protocol (mailbox layout, command table,
  versioning rules) and SDK usage examples.

## Testing strategy

- Unit tests: `AgentClient` protocol logic against a fake `Backend` (in-memory
  mailbox) — covers handshake, timeout, version mismatch, each command.
- Backend tests: `write_mem32`/`read_mem32` round trip on QEMU gdbstub
  (mirrors existing gdbstub tests); hardware path exercised live.
- e2e: `e2e_scripted_uart.rs` on QEMU (CI) + live hardware this session.
- Validator/compiler tests: scripted schema accept/reject cases.

## Risks / open verification

- **QEMU `netduinoplus2` USART → `-serial` routing must be confirmed early.**
  If the machine doesn't surface USART2 to a host-readable chardev, the
  VCP-based RX/TX round trip can't run on QEMU; in that case the loopback
  degrades to hardware-only, while the mailbox GPIO/register commands remain
  QEMU-testable. Verify before building the fixture.
- **Live-hardware SWO/RSP flakiness** (per M6 notes): board must be connected;
  reap stray `openocd` between runs.
- **Mailbox vs running core:** memory reads/writes require the standard
  `halt`/op/`resume` on hardware; confirm the agent's poll loop tolerates the
  brief halts (it does — state lives in RAM, not registers).

## Out of scope

- Additional command-protocol versions beyond v1.
- Ledger/CI/lockstep (M8–M10).
- Non-UART scripted fixtures beyond the loopback example.

## Acceptance mapping (issue #21 M7)

- [ ] Device test-agent spec → §1 (mailbox protocol v1, commands, test-mode gate)
- [ ] Host SDK `nucleus_test_sdk` → §3
- [ ] Example fixture: UART loopback on both backends → §5
- [ ] M6 integration `type = "scripted"` → §4
- [ ] Documentation: protocol versioning + SDK usage → §5
