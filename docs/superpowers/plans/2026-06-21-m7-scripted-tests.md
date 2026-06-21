# M7 Scripted Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a host-driven scripted-test path: a RAM-mailbox device test-agent (firmware), a Rust host SDK (`nucleus_test_sdk`), a two-channel UART loopback fixture on both backends, and `[[test]] type = "scripted"` wiring into `nucleus test`.

**Architecture:** Both HIL backends already do debugger memory access (OpenOCD telnet `mdw` on hardware, gdbstub `m` on QEMU). M7 adds the `write` half and layers a fixed-address RAM mailbox protocol on top in a new SDK crate. The device agent polls the mailbox and drives GPIO/registers/USART2; the host drives the agent over SWD and reads/writes the UART over the ST-Link VCP. Scripted `[[test]]` entries shell out to `cargo test`.

**Tech Stack:** Rust (workspace crates), C + arm-none-eabi-gcc (fixture firmware), QEMU `qemu-system-arm -M netduinoplus2`, OpenOCD + ST-Link, `serialport` crate (SDK-only).

## Global Constraints

- MCUs: F446RE + F411RE only. Fixture targets F411RE (Cortex-M4).
- Local tool: no cloud, no uploads.
- Codegen calls only stock HAL — N/A here (fixtures are hand-written, not codegen output).
- `nucleus check` / `nucleus test` exit non-zero on conflict / test failure.
- Backends never run a config the verifier flagged: every `Backend::start` calls `preflight::gate` first (already true; new methods don't change this).
- `nucleus-itm` stays zero-dependency. The new `serialport` dep lives ONLY in `nucleus-test-sdk`.
- Workspace lints: `cargo clippy -D warnings` must pass (`make lint`).
- Caveman mode is for chat only — all code, comments, commits in normal English.
- Fixtures: CI has no `arm-none-eabi-gcc`; commit prebuilt `.elf`/`.bin`, `build.sh` is hand-run only (mirror `tests/fixtures/blink_itm/`).

---

### Task 1: gdbstub `write_memory` (QEMU memory-write primitive)

**Files:**
- Modify: `crates/nucleus-hil/src/gdbstub.rs` (add method + test near existing `read_memory` at :50 and its mock-server tests at :176)

**Interfaces:**
- Consumes: existing `GdbStub::{send_packet, read_packet}`, `decode_hex`.
- Produces: `pub async fn GdbStub::write_memory(&mut self, addr: u32, bytes: &[u8]) -> Result<(), HilError>` — writes via the `M<addr>,<len>:<hex>` packet; `Err(HilError::Protocol)` if the stub replies anything but `OK`.

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)] mod tests` of `gdbstub.rs`, mirror the existing mock-server pattern (`reads_memory_from_well_formed_reply`). Add:

```rust
#[tokio::test]
async fn writes_memory_and_accepts_ok_reply() {
    // Mock server: accept the M packet, ack '+', reply "$OK#<sum>".
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // read the inbound packet bytes (best-effort, we don't parse here)
        let mut buf = [0u8; 64];
        let _ = sock.read(&mut buf).await.unwrap();
        sock.write_all(b"+").await.unwrap();
        let body = "OK";
        let sum = body.bytes().fold(0u8, |a, b| a.wrapping_add(b));
        let framed = format!("${body}#{sum:02x}");
        sock.write_all(framed.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
    });

    let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
    stub.write_memory(0x2000_0000, &[0x67, 0x41, 0x54, 0x4e]).await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn write_memory_surfaces_error_reply() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 64];
        let _ = sock.read(&mut buf).await.unwrap();
        sock.write_all(b"+").await.unwrap();
        let framed = "$E01#"; // body "E01"
        let sum = "E01".bytes().fold(0u8, |a, b| a.wrapping_add(b));
        sock.write_all(format!("{framed}{sum:02x}").as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
    });
    let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
    let err = stub.write_memory(0x2000_0000, &[1, 2, 3, 4]).await.unwrap_err();
    assert!(matches!(err, HilError::Protocol(_)));
}
```

Ensure `use tokio::io::{AsyncReadExt, AsyncWriteExt};` is in scope in the test module (the existing tests already use the mock-server pattern — match their imports).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nucleus-hil --lib gdbstub::tests::writes_memory_and_accepts_ok_reply`
Expected: FAIL — `no method named write_memory`.

- [ ] **Step 3: Implement `write_memory`**

Add to `impl GdbStub`, right after `read_memory`:

```rust
/// Write `bytes` to target memory at `addr` via the `M` packet
/// (`$M<addr>,<len>:<hex bytes>#<checksum>`). Stub must reply `OK`.
pub async fn write_memory(&mut self, addr: u32, bytes: &[u8]) -> Result<(), HilError> {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        hex.push_str(&format!("{b:02x}"));
    }
    let payload = format!("M{addr:x},{:x}:{hex}", bytes.len());
    self.send_packet(&payload).await?;
    let reply = self.read_packet().await?;
    if reply == "OK" {
        Ok(())
    } else {
        Err(HilError::Protocol(format!(
            "memory write rejected by target: {reply}"
        )))
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nucleus-hil --lib gdbstub::tests`
Expected: PASS (both new tests + existing ones).

- [ ] **Step 5: Commit**

```bash
git add crates/nucleus-hil/src/gdbstub.rs
git commit -m "feat(nucleus-hil): add GdbStub::write_memory (M-packet)"
```

---

### Task 2: `Backend` memory primitives (`read_mem32` / `write_mem32`)

**Files:**
- Modify: `crates/nucleus-hil/src/backend.rs:137-161` (trait)
- Modify: `crates/nucleus-hil/src/qemu/mod.rs` (impl, near `register` at :171)
- Modify: `crates/nucleus-hil/src/hardware/mod.rs` (impl; `read_memory` private fn at :256, add `write_memory`)
- Modify: `crates/nucleus-hil/src/lib.rs:160-211` (the `CountingBackend` test fake — must implement the new methods)

**Interfaces:**
- Produces (trait methods, both default-free, required):
  - `fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError>`
  - `fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError>`
- Consumes: QEMU `GdbStub::{interrupt, read_memory, write_memory, continue_execution}`; hardware telnet helpers `telnet_write_line`, `read_until_value`, `parse_mdw_word`.

- [ ] **Step 1: Add the trait methods (compile-driver "test")**

In `backend.rs`, inside `pub trait Backend`, after `fn register(...)`:

```rust
    /// Read one 32-bit little-endian word at absolute address `addr`.
    fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError>;

    /// Write one 32-bit little-endian word `value` to absolute address `addr`.
    fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError>;
```

- [ ] **Step 2: Run build to verify it fails**

Run: `cargo build -p nucleus-hil`
Expected: FAIL — `not all trait items implemented` for `QemuBackend`, `HardwareBackend`, and `CountingBackend`.

- [ ] **Step 3: Implement on `QemuBackend`**

In `qemu/mod.rs`, add to the `impl Backend for QemuBackend` block (after `register`):

```rust
    fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError> {
        let stub = self
            .stub
            .as_mut()
            .ok_or_else(|| HilError::Protocol("backend not started".to_string()))?;
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        let result = runtime.block_on(async {
            stub.interrupt().await?;
            let r = stub.read_memory(addr, 4).await;
            stub.continue_execution().await?;
            r
        });
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(err) => return Err(self.record_failure(err)),
        };
        bytes
            .try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| HilError::Protocol("expected 4-byte read".to_string()))
    }

    fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError> {
        let stub = self
            .stub
            .as_mut()
            .ok_or_else(|| HilError::Protocol("backend not started".to_string()))?;
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        let result = runtime.block_on(async {
            stub.interrupt().await?;
            let r = stub.write_memory(addr, &value.to_le_bytes()).await;
            stub.continue_execution().await?;
            r
        });
        result.map_err(|err| self.record_failure(err))
    }
```

Then simplify `register` to delegate (keeps one read path):

```rust
    fn register(&mut self, peripheral: &str, offset: u32) -> Result<u32, HilError> {
        let base = match peripheral {
            "TIM2" => TIM2_BASE,
            other => {
                return Err(HilError::NotObservable {
                    peripheral: other.to_string(),
                })
            }
        };
        self.read_mem32(base + offset)
    }
```

(Confirm `record_failure` exists on `QemuBackend`; the existing `register` uses it via the match — if `register`'s old body called `self.record_failure`, reuse it; otherwise the existing inline error path becomes `read_mem32`'s.)

- [ ] **Step 4: Implement on `HardwareBackend`**

In `hardware/mod.rs`, add a `write_memory` private method mirroring `read_memory` (:256), using `mww`:

```rust
    /// Write one 32-bit word `value` at `addr` over OpenOCD's telnet console:
    /// `halt`, `mww 0x<addr> 0x<value>`, `resume`. See the module doc comment
    /// for why this uses telnet rather than the gdbserver.
    fn write_memory(&mut self, addr: u32, value: u32) -> Result<(), HilError> {
        if !self.started {
            return Err(HilError::Protocol("backend not started".to_string()));
        }
        let runtime = self.runtime.as_ref().expect("runtime set in start()");
        let telnet_addr = format!("127.0.0.1:{}", self.telnet_port);
        let result: Result<(), HilError> = runtime.block_on(async {
            let mut conn = TcpStream::connect(&telnet_addr).await?;
            telnet_write_line(&mut conn, "halt").await?;
            tokio::time::sleep(Duration::from_millis(20)).await;
            telnet_write_line(&mut conn, &format!("mww 0x{addr:08x} 0x{value:08x}")).await?;
            // Brief settle so OpenOCD applies the write before we resume.
            tokio::time::sleep(Duration::from_millis(10)).await;
            telnet_write_line(&mut conn, "resume").await?;
            Ok(())
        });
        result.map_err(|err| self.record_failure(err))
    }
```

Then add the trait methods to `impl Backend for HardwareBackend` (the existing `register` already calls `self.read_memory(addr)`-style logic — reuse it):

```rust
    fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError> {
        self.read_memory(addr)
    }

    fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError> {
        self.write_memory(addr, value)
    }
```

(If `HardwareBackend::read_memory` is currently private and `register` wraps it, leave `register` as-is; `read_mem32` just exposes the same word read at an absolute address.)

- [ ] **Step 5: Implement on the `CountingBackend` test fake**

In `lib.rs` test module, add to `impl Backend for CountingBackend`:

```rust
    fn read_mem32(&mut self, _addr: u32) -> Result<u32, HilError> {
        self.calls.set(self.calls.get() + 1);
        Ok(0)
    }

    fn write_mem32(&mut self, _addr: u32, _value: u32) -> Result<(), HilError> {
        self.calls.set(self.calls.get() + 1);
        Ok(())
    }
```

- [ ] **Step 6: Run build + existing tests**

Run: `cargo test -p nucleus-hil --lib`
Expected: PASS (all existing tests still green; nothing yet exercises the new methods beyond compilation + the fake).

- [ ] **Step 7: Commit**

```bash
git add crates/nucleus-hil/src/backend.rs crates/nucleus-hil/src/qemu/mod.rs crates/nucleus-hil/src/hardware/mod.rs crates/nucleus-hil/src/lib.rs
git commit -m "feat(nucleus-hil): add Backend::{read_mem32,write_mem32} on both backends"
```

---

### Task 3: `nucleus-test-sdk` crate — mailbox protocol (`AgentClient`)

**Files:**
- Create: `crates/nucleus-test-sdk/Cargo.toml`
- Create: `crates/nucleus-test-sdk/src/lib.rs`
- Create: `crates/nucleus-test-sdk/src/protocol.rs` (constants + command ids)
- Create: `crates/nucleus-test-sdk/src/agent.rs` (`AgentClient`)
- Modify: `Cargo.toml` (workspace root — add the crate to `members`)

**Interfaces:**
- Consumes: `nucleus_hil::backend::{Backend, HilError}`.
- Produces:
  - `nucleus_test_sdk::protocol::{MAILBOX_BASE, MAGIC, PROTO_VERSION}` and command-id consts.
  - `pub enum SdkError { Hil(HilError), BadMagic(u32), VersionMismatch{found,expected}, AgentError{cmd:u32}, Timeout }`
  - `pub struct AgentClient<'a> { backend: &'a mut dyn Backend, base: u32, poll_timeout: Duration }`
  - `AgentClient::new(&mut dyn Backend) -> Self` (base = `MAILBOX_BASE`)
  - `connect(&mut self) -> Result<u32, SdkError>` (verifies magic+version, returns version)
  - `ping(&mut self) -> Result<u32, SdkError>`
  - `set_gpio(&mut self, port: nucleus_db::Port, pin: u8, level: bool) -> Result<(), SdkError>`
  - `read_gpio(&mut self, port: nucleus_db::Port, pin: u8) -> Result<bool, SdkError>`
  - `read_register(&mut self, addr: u32) -> Result<u32, SdkError>`
  - `uart_tx(&mut self, byte: u8) -> Result<(), SdkError>`
  - `uart_rx_poll(&mut self) -> Result<Option<u8>, SdkError>`

- [ ] **Step 1: Scaffold the crate**

Create `crates/nucleus-test-sdk/Cargo.toml`:

```toml
[package]
name = "nucleus-test-sdk"
description = "Nucleus M7 host SDK: drive the device test-agent over the SWD RAM mailbox and the UART VCP"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
keywords = ["stm32", "hil", "testing", "swd"]
categories = ["embedded", "development-tools::testing"]

[dependencies]
nucleus-hil = { path = "../nucleus-hil", version = "0.1.0" }
nucleus-db = { path = "../nucleus-db", version = "0.1.0" }
serialport = "4"

[lints]
workspace = true
```

Add `"crates/nucleus-test-sdk"` to the workspace `members` array in the root `Cargo.toml` (keep alphabetical/existing ordering consistent with siblings).

`crates/nucleus-test-sdk/src/lib.rs`:

```rust
//! Nucleus M7 host SDK (`nucleus_test_sdk`): the power-user escape hatch for
//! scripted, host-driven HIL tests. Drives the on-device test-agent over a
//! fixed-address RAM mailbox (via any [`nucleus_hil::backend::Backend`]'s
//! `read_mem32`/`write_mem32`) and the device UART over the ST-Link VCP.

pub mod agent;
pub mod protocol;
pub mod serial;

pub use agent::{AgentClient, SdkError};
pub use serial::Serial;
```

(`serial` module is Task 4 — create a stub `pub mod serial {}` placeholder file now so the crate compiles, or defer the `pub mod serial;` line until Task 4. Defer it: in this task, `lib.rs` has only `pub mod agent;` and `pub mod protocol;` plus the two re-exports for those.)

Corrected `lib.rs` for THIS task (no serial yet):

```rust
//! Nucleus M7 host SDK ... (doc as above)
pub mod agent;
pub mod protocol;

pub use agent::{AgentClient, SdkError};
```

- [ ] **Step 2: Write `protocol.rs`**

```rust
//! Wire constants for the device test-agent RAM mailbox (protocol v1).
//! Mirrored byte-for-byte by the agent firmware in
//! `nucleus-hil/tests/fixtures/agent_loopback/agent.c`.

/// Mailbox base address: start of SRAM, pinned by the agent's linker script
/// (`.nucleus_agent` section). The host needs no ELF symbol lookup.
pub const MAILBOX_BASE: u32 = 0x2000_0000;

/// `'NTAg'` little-endian — agent writes this once initialized.
pub const MAGIC: u32 = 0x4E54_4167;
pub const PROTO_VERSION: u32 = 1;

// Field offsets from MAILBOX_BASE.
pub const OFF_MAGIC: u32 = 0x00;
pub const OFF_VERSION: u32 = 0x04;
pub const OFF_SEQ: u32 = 0x08;
pub const OFF_CMD: u32 = 0x0C;
pub const OFF_ARG0: u32 = 0x10;
pub const OFF_ARG1: u32 = 0x14;
pub const OFF_STATUS: u32 = 0x18;
pub const OFF_RESP: u32 = 0x1C;

// Status values.
pub const STATUS_IDLE: u32 = 0;
pub const STATUS_BUSY: u32 = 1;
pub const STATUS_DONE: u32 = 2;
pub const STATUS_ERR: u32 = 3;

// Command ids.
pub const CMD_PING: u32 = 0;
pub const CMD_SET_GPIO: u32 = 1;
pub const CMD_READ_GPIO: u32 = 2;
pub const CMD_READ_REG: u32 = 3;
pub const CMD_UART_TX: u32 = 4;
pub const CMD_UART_RX_POLL: u32 = 5;

/// `UART_RX_POLL` resp sentinel meaning "no byte available".
pub const RX_NONE: u32 = 0xFFFF_FFFF;

/// Encode a GPIO port+pin into a command argument: `(port_index << 8) | pin`.
pub fn encode_pin(port: nucleus_db::Port, pin: u8) -> u32 {
    ((port as u32) << 8) | (pin as u32)
}
```

Note: confirm `nucleus_db::Port` is `#[repr]`/castable to a stable index. If `Port` is a plain `enum { A, B, ... }`, `port as u32` gives its discriminant — the agent firmware must use the SAME ordering (document A=0,B=1,C=2,... in the agent and in the docs page, Task 9). If `Port` is not `Copy`/`as`-castable, add a small `match` mapping instead. Verify against `nucleus-db` before relying on the cast.

- [ ] **Step 3: Write the failing `AgentClient` test (against an in-memory fake Backend)**

Create `crates/nucleus-test-sdk/src/agent.rs` with the test module first; the fake `Backend` is a HashMap-backed RAM plus a tiny simulated agent that, whenever `write_mem32` sets `OFF_STATUS` to `STATUS_BUSY`, immediately executes the command and writes `resp` + `STATUS_DONE`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_hil::backend::{
        Backend, BackendKind, FirmwareArtifact, HilError, ItmEvent, RunResult, Sample,
    };
    use crate::protocol::*;
    use std::collections::HashMap;
    use std::time::Duration;

    /// RAM + a simulated agent. On any write that sets STATUS=BUSY, runs the
    /// command and posts DONE/resp, so the host handshake can be tested with
    /// no real device.
    struct FakeDevice {
        ram: HashMap<u32, u32>,
        gpio: HashMap<u32, bool>, // keyed by encoded pin
        rx_queue: std::collections::VecDeque<u8>,
        tx_log: Vec<u8>,
    }

    impl FakeDevice {
        fn new() -> Self {
            let mut ram = HashMap::new();
            ram.insert(MAILBOX_BASE + OFF_MAGIC, MAGIC);
            ram.insert(MAILBOX_BASE + OFF_VERSION, PROTO_VERSION);
            ram.insert(MAILBOX_BASE + OFF_STATUS, STATUS_IDLE);
            Self {
                ram,
                gpio: HashMap::new(),
                rx_queue: Default::default(),
                tx_log: Vec::new(),
            }
        }
        fn run_command(&mut self) {
            let cmd = *self.ram.get(&(MAILBOX_BASE + OFF_CMD)).unwrap_or(&0);
            let a0 = *self.ram.get(&(MAILBOX_BASE + OFF_ARG0)).unwrap_or(&0);
            let mut resp = 0u32;
            match cmd {
                CMD_PING => resp = PROTO_VERSION,
                CMD_SET_GPIO => {
                    let level = *self.ram.get(&(MAILBOX_BASE + OFF_ARG1)).unwrap_or(&0) != 0;
                    self.gpio.insert(a0, level);
                }
                CMD_READ_GPIO => resp = u32::from(*self.gpio.get(&a0).unwrap_or(&false)),
                CMD_READ_REG => resp = *self.ram.get(&a0).unwrap_or(&0),
                CMD_UART_TX => self.tx_log.push(a0 as u8),
                CMD_UART_RX_POLL => {
                    resp = self.rx_queue.pop_front().map(u32::from).unwrap_or(RX_NONE)
                }
                _ => {
                    self.ram.insert(MAILBOX_BASE + OFF_RESP, 0);
                    self.ram.insert(MAILBOX_BASE + OFF_STATUS, STATUS_ERR);
                    return;
                }
            }
            self.ram.insert(MAILBOX_BASE + OFF_RESP, resp);
            self.ram.insert(MAILBOX_BASE + OFF_STATUS, STATUS_DONE);
        }
    }

    impl Backend for FakeDevice {
        fn name(&self) -> BackendKind { BackendKind::Qemu }
        fn start(&mut self, _f: &FirmwareArtifact, _r: &nucleus_compiler::CheckReport) -> Result<(), HilError> { Ok(()) }
        fn pin(&mut self, _p: nucleus_db::Port, _n: u8) -> Result<bool, HilError> { Ok(false) }
        fn register(&mut self, _p: &str, _o: u32) -> Result<u32, HilError> { Ok(0) }
        fn await_itm_event(&mut self, _t: Duration) -> Result<Option<ItmEvent>, HilError> { Ok(None) }
        fn sample(&mut self, _d: Duration) -> Result<Sample, HilError> { unimplemented!() }
        fn finish(&mut self) -> RunResult { unimplemented!() }
        fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError> {
            Ok(*self.ram.get(&addr).unwrap_or(&0))
        }
        fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError> {
            self.ram.insert(addr, value);
            if addr == MAILBOX_BASE + OFF_STATUS && value == STATUS_BUSY {
                self.run_command();
            }
            Ok(())
        }
    }

    #[test]
    fn connect_verifies_magic_and_version() {
        let mut dev = FakeDevice::new();
        let mut c = AgentClient::new(&mut dev);
        assert_eq!(c.connect().unwrap(), PROTO_VERSION);
    }

    #[test]
    fn connect_rejects_version_mismatch() {
        let mut dev = FakeDevice::new();
        dev.ram.insert(MAILBOX_BASE + OFF_VERSION, 99);
        let mut c = AgentClient::new(&mut dev);
        assert!(matches!(c.connect(), Err(SdkError::VersionMismatch { .. })));
    }

    #[test]
    fn ping_returns_version() {
        let mut dev = FakeDevice::new();
        let mut c = AgentClient::new(&mut dev);
        c.connect().unwrap();
        assert_eq!(c.ping().unwrap(), PROTO_VERSION);
    }

    #[test]
    fn set_then_read_gpio_roundtrips() {
        let mut dev = FakeDevice::new();
        let mut c = AgentClient::new(&mut dev);
        c.connect().unwrap();
        c.set_gpio(nucleus_db::Port::A, 5, true).unwrap();
        assert!(c.read_gpio(nucleus_db::Port::A, 5).unwrap());
    }

    #[test]
    fn uart_rx_poll_returns_none_then_byte() {
        let mut dev = FakeDevice::new();
        dev.rx_queue.push_back(0x42);
        let mut c = AgentClient::new(&mut dev);
        c.connect().unwrap();
        assert_eq!(c.uart_rx_poll().unwrap(), Some(0x42));
        assert_eq!(c.uart_rx_poll().unwrap(), None);
    }

    #[test]
    fn uart_tx_reaches_device() {
        let mut dev = FakeDevice::new();
        {
            let mut c = AgentClient::new(&mut dev);
            c.connect().unwrap();
            c.uart_tx(0x5A).unwrap();
        }
        assert_eq!(dev.tx_log, vec![0x5A]);
    }
}
```

`FakeDevice` needs `nucleus-compiler` for the `CheckReport` type in `start`'s signature — add `nucleus-compiler = { path = "../nucleus-compiler", version = "0.1.0" }` to `[dev-dependencies]` in this crate's `Cargo.toml`.

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p nucleus-test-sdk`
Expected: FAIL — `AgentClient`, `SdkError` not found.

- [ ] **Step 5: Implement `AgentClient`**

Above the test module in `agent.rs`:

```rust
use std::time::{Duration, Instant};

use nucleus_db::Port;
use nucleus_hil::backend::{Backend, HilError};

use crate::protocol::*;

/// Errors talking to the device test-agent.
#[derive(Debug)]
pub enum SdkError {
    Hil(HilError),
    BadMagic(u32),
    VersionMismatch { found: u32, expected: u32 },
    AgentError { cmd: u32 },
    Timeout { cmd: u32 },
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdkError::Hil(e) => write!(f, "backend error: {e}"),
            SdkError::BadMagic(m) => write!(f, "mailbox magic mismatch: got 0x{m:08x}"),
            SdkError::VersionMismatch { found, expected } => {
                write!(f, "agent protocol v{found}, SDK expects v{expected}")
            }
            SdkError::AgentError { cmd } => write!(f, "agent reported error for command {cmd}"),
            SdkError::Timeout { cmd } => write!(f, "agent did not complete command {cmd} in time"),
        }
    }
}
impl std::error::Error for SdkError {}
impl From<HilError> for SdkError {
    fn from(e: HilError) -> Self { SdkError::Hil(e) }
}

/// Host-side driver for the on-device test-agent's RAM mailbox.
pub struct AgentClient<'a> {
    backend: &'a mut dyn Backend,
    base: u32,
    poll_timeout: Duration,
}

impl<'a> AgentClient<'a> {
    pub fn new(backend: &'a mut dyn Backend) -> Self {
        Self { backend, base: MAILBOX_BASE, poll_timeout: Duration::from_millis(500) }
    }

    /// Verify the agent is alive: read magic + version.
    pub fn connect(&mut self) -> Result<u32, SdkError> {
        let magic = self.backend.read_mem32(self.base + OFF_MAGIC)?;
        if magic != MAGIC {
            return Err(SdkError::BadMagic(magic));
        }
        let version = self.backend.read_mem32(self.base + OFF_VERSION)?;
        if version != PROTO_VERSION {
            return Err(SdkError::VersionMismatch { found: version, expected: PROTO_VERSION });
        }
        Ok(version)
    }

    /// Issue one command and block until the agent posts DONE/ERR (or times out).
    fn issue(&mut self, cmd: u32, arg0: u32, arg1: u32) -> Result<u32, SdkError> {
        self.backend.write_mem32(self.base + OFF_ARG0, arg0)?;
        self.backend.write_mem32(self.base + OFF_ARG1, arg1)?;
        self.backend.write_mem32(self.base + OFF_CMD, cmd)?;
        let seq = self.backend.read_mem32(self.base + OFF_SEQ)?;
        self.backend.write_mem32(self.base + OFF_SEQ, seq.wrapping_add(1))?;
        self.backend.write_mem32(self.base + OFF_STATUS, STATUS_BUSY)?;

        let deadline = Instant::now() + self.poll_timeout;
        loop {
            let status = self.backend.read_mem32(self.base + OFF_STATUS)?;
            match status {
                STATUS_DONE => return Ok(self.backend.read_mem32(self.base + OFF_RESP)?),
                STATUS_ERR => return Err(SdkError::AgentError { cmd }),
                _ if Instant::now() >= deadline => return Err(SdkError::Timeout { cmd }),
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    pub fn ping(&mut self) -> Result<u32, SdkError> { self.issue(CMD_PING, 0, 0) }

    pub fn set_gpio(&mut self, port: Port, pin: u8, level: bool) -> Result<(), SdkError> {
        self.issue(CMD_SET_GPIO, encode_pin(port, pin), u32::from(level)).map(|_| ())
    }

    pub fn read_gpio(&mut self, port: Port, pin: u8) -> Result<bool, SdkError> {
        Ok(self.issue(CMD_READ_GPIO, encode_pin(port, pin), 0)? != 0)
    }

    pub fn read_register(&mut self, addr: u32) -> Result<u32, SdkError> {
        self.issue(CMD_READ_REG, addr, 0)
    }

    pub fn uart_tx(&mut self, byte: u8) -> Result<(), SdkError> {
        self.issue(CMD_UART_TX, u32::from(byte), 0).map(|_| ())
    }

    pub fn uart_rx_poll(&mut self) -> Result<Option<u8>, SdkError> {
        let r = self.issue(CMD_UART_RX_POLL, 0, 0)?;
        Ok(if r == RX_NONE { None } else { Some(r as u8) })
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p nucleus-test-sdk`
Expected: PASS (all six tests).

- [ ] **Step 7: Commit**

```bash
git add crates/nucleus-test-sdk Cargo.toml Cargo.lock
git commit -m "feat(nucleus-test-sdk): mailbox AgentClient + protocol v1"
```

---

### Task 4: `nucleus-test-sdk` Serial helper (VCP / QEMU TCP)

**Files:**
- Create: `crates/nucleus-test-sdk/src/serial.rs`
- Modify: `crates/nucleus-test-sdk/src/lib.rs` (add `pub mod serial;` + `pub use serial::Serial;`)

**Interfaces:**
- Produces:
  - `pub enum Serial { Tcp(std::net::TcpStream), Port(Box<dyn serialport::SerialPort>) }`
  - `Serial::open_tcp(addr: &str) -> std::io::Result<Serial>`
  - `Serial::open_device(path: &str, baud: u32) -> Result<Serial, serialport::Error>`
  - `write_byte(&mut self, b: u8) -> std::io::Result<()>`
  - `read_byte(&mut self, timeout: Duration) -> std::io::Result<Option<u8>>` (Ok(None) on timeout)

- [ ] **Step 1: Write the failing test (TCP loopback)**

In `serial.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn tcp_roundtrips_a_byte() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // echo one byte back
            let mut buf = [0u8; 1];
            use std::io::Read;
            sock.read_exact(&mut buf).unwrap();
            sock.write_all(&buf).unwrap();
        });
        let mut s = Serial::open_tcp(&addr.to_string()).unwrap();
        s.write_byte(0x39).unwrap();
        assert_eq!(s.read_byte(Duration::from_secs(1)).unwrap(), Some(0x39));
        server.join().unwrap();
    }

    #[test]
    fn read_byte_times_out_to_none() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = std::thread::spawn(move || { let _ = listener.accept(); std::thread::sleep(Duration::from_millis(200)); });
        let mut s = Serial::open_tcp(&addr.to_string()).unwrap();
        assert_eq!(s.read_byte(Duration::from_millis(50)).unwrap(), None);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nucleus-test-sdk serial`
Expected: FAIL — `Serial` not found.

- [ ] **Step 3: Implement `serial.rs`**

```rust
//! Host-side UART access for scripted tests: the ST-Link Virtual COM Port on
//! real hardware (`/dev/ttyACM*`), or QEMU's `-serial tcp` socket on the
//! emulator. One byte at a time, with a read timeout that degrades to
//! `Ok(None)` rather than erroring.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub enum Serial {
    Tcp(TcpStream),
    Port(Box<dyn serialport::SerialPort>),
}

impl Serial {
    pub fn open_tcp(addr: &str) -> std::io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Serial::Tcp(stream))
    }

    pub fn open_device(path: &str, baud: u32) -> Result<Self, serialport::Error> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(50))
            .open()?;
        Ok(Serial::Port(port))
    }

    pub fn write_byte(&mut self, b: u8) -> std::io::Result<()> {
        match self {
            Serial::Tcp(s) => { s.write_all(&[b])?; s.flush() }
            Serial::Port(p) => { p.write_all(&[b])?; p.flush() }
        }
    }

    /// Read one byte, waiting up to `timeout`. `Ok(None)` means nothing
    /// arrived in time (not an error).
    pub fn read_byte(&mut self, timeout: Duration) -> std::io::Result<Option<u8>> {
        let mut buf = [0u8; 1];
        match self {
            Serial::Tcp(s) => {
                s.set_read_timeout(Some(timeout))?;
                match s.read(&mut buf) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(buf[0])),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
                    Err(e) => Err(e),
                }
            }
            Serial::Port(p) => {
                p.set_timeout(timeout).ok();
                match p.read(&mut buf) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(buf[0])),
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
                    Err(e) => Err(e),
                }
            }
        }
    }
}
```

Add to `lib.rs`: `pub mod serial;` and `pub use serial::Serial;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nucleus-test-sdk`
Expected: PASS (agent + serial tests).

- [ ] **Step 5: Commit**

```bash
git add crates/nucleus-test-sdk/src/serial.rs crates/nucleus-test-sdk/src/lib.rs
git commit -m "feat(nucleus-test-sdk): Serial helper (VCP + QEMU TCP)"
```

---

### Task 5: Compiler — `type`/`script` schema, validation, scripted `CompiledTest`

**Files:**
- Modify: `crates/nucleus-compiler/src/config.rs:176-183` (`TestCase`)
- Modify: `crates/nucleus-compiler/src/solver.rs:334-386` (validation loop)
- Modify: `crates/nucleus-compiler/src/lib.rs:184-244` (`CompiledTest` + `test_plan`)

**Interfaces:**
- Consumes: existing `TestCase`, `assertion::parse`, `Conflict::InvalidTest`.
- Produces:
  - `TestCase` gains `pub kind: Option<String>` (serde `#[serde(rename = "type")]`) and `pub script: Option<String>`.
  - `nucleus_compiler::TestBody` enum: `Declarative(Assertion)` | `Scripted { script: String }`.
  - `CompiledTest` field change: `pub body: TestBody` replaces `pub assertion: Assertion` (keeps `name`, `timeout`, `backend`).
  - `pub fn CompiledTest::assertion(&self) -> Option<&Assertion>` convenience (so existing declarative consumers read it ergonomically).

- [ ] **Step 1: Write failing compiler tests**

In `lib.rs` test module (near the existing `test_plan` tests around :507/:624) add:

```rust
#[test]
fn scripted_test_compiles_to_scripted_body() {
    let toml = r#"
[[test]]
name = "uart_loopback"
type = "scripted"
script = "uart_loopback"
backend = "both"
"#;
    let plan = test_plan(toml).unwrap().unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].name, "uart_loopback");
    assert!(matches!(
        &plan[0].body,
        nucleus_compiler::TestBody::Scripted { script } if script == "uart_loopback"
    ));
}

#[test]
fn scripted_test_without_script_is_invalid() {
    let toml = r#"
[[test]]
name = "bad"
type = "scripted"
"#;
    let conflicts = test_plan(toml).unwrap().unwrap_err();
    assert!(conflicts.iter().any(|c| matches!(c, Conflict::InvalidTest { .. })));
}

#[test]
fn declarative_test_with_script_is_invalid() {
    let toml = r#"
[[test]]
name = "bad"
assertion = "trace event \"x\" within 50ms"
script = "nope"
"#;
    let conflicts = test_plan(toml).unwrap().unwrap_err();
    assert!(conflicts.iter().any(|c| matches!(c, Conflict::InvalidTest { .. })));
}
```

(Use whatever assertion-string syntax the existing M6 tests use for a valid declarative example — match an existing passing test's string verbatim.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p nucleus-compiler scripted_test_compiles_to_scripted_body`
Expected: FAIL — `TestBody` not found / `body` field missing.

- [ ] **Step 3: Extend `TestCase`**

In `config.rs`, inside `struct TestCase` (after `backend`):

```rust
    /// `"declarative"` (default, M6) or `"scripted"` (M7). TOML key is `type`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// For `type = "scripted"`: the cargo test name to run. Ignored otherwise.
    #[serde(default)]
    pub script: Option<String>,
```

Also make `assertion` optional so a scripted block needn't supply one. Change `pub assertion: String` to `pub assertion: Option<String>` ... **but** that ripples through M6 code. Instead keep `assertion: String` with `#[serde(default)]` so it defaults to `""` when omitted, and treat `""` as "no assertion" in the solver/test_plan. Choose the `#[serde(default)]` route to minimize churn:

```rust
    #[serde(default)]
    pub assertion: String,
```

- [ ] **Step 4: Update solver validation**

In `solver.rs`, replace the per-test validation body (the loop at :334) so it branches on kind. Pseudocode-to-code:

```rust
    for test in &config.test {
        let is_scripted = test.kind.as_deref() == Some("scripted");
        let kind_valid = match test.kind.as_deref() {
            None | Some("declarative") | Some("scripted") => true,
            Some(other) => {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: format!("type must be \"declarative\" or \"scripted\", got {other:?}"),
                });
                false
            }
        };
        if !kind_valid { continue; }

        if is_scripted {
            if test.script.as_deref().unwrap_or("").is_empty() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "scripted test requires a non-empty `script`".to_string(),
                });
            }
            if !test.assertion.is_empty() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "scripted test must not set `assertion`".to_string(),
                });
            }
            // backend value still validated below (shared)
        } else {
            // declarative: existing M6 validation (assertion parse + subject check)
            if test.script.is_some() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "declarative test must not set `script`".to_string(),
                });
                continue;
            }
            if test.assertion.is_empty() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "declarative test requires an `assertion`".to_string(),
                });
                continue;
            }
            // ... existing assertion::parse + subject_invalid block unchanged ...
        }

        // shared backend validation (existing block at :376) applies to both.
        if let Some(backend) = &test.backend {
            if backend != "qemu" && backend != "hardware" && backend != "both" {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: format!("backend must be \"qemu\", \"hardware\", or \"both\", got {backend:?}"),
                });
            }
        }
    }
```

Keep the existing assertion-parse + `subject_invalid` logic verbatim inside the `else` (declarative) branch.

- [ ] **Step 5: Update `CompiledTest` + `test_plan`**

In `lib.rs`:

```rust
/// A compiled test's payload: a declarative assertion (M6) or a scripted
/// cargo-test pointer (M7).
#[derive(Debug, Clone, PartialEq)]
pub enum TestBody {
    Declarative(Assertion),
    Scripted { script: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTest {
    pub name: String,
    pub body: TestBody,
    pub timeout: std::time::Duration,
    pub backend: BackendSelect,
}

impl CompiledTest {
    /// The declarative assertion, if this is a declarative test.
    pub fn assertion(&self) -> Option<&Assertion> {
        match &self.body {
            TestBody::Declarative(a) => Some(a),
            TestBody::Scripted { .. } => None,
        }
    }
}
```

In `test_plan`'s map closure, build `body` from kind:

```rust
        .map(|t| {
            let backend = match t.backend.as_deref() {
                None | Some("both") => BackendSelect::Both,
                Some("qemu") => BackendSelect::Qemu,
                Some("hardware") => BackendSelect::Hardware,
                Some(other) => unreachable!("solve() validated backend: {other:?}"),
            };
            let body = if t.kind.as_deref() == Some("scripted") {
                TestBody::Scripted {
                    script: t.script.clone().expect("solve() validated scripted script present"),
                }
            } else {
                TestBody::Declarative(assertion::parse(&t.assertion).expect(
                    "solve() validated declarative assertion",
                ))
            };
            CompiledTest { name: t.name.clone(), body, timeout: std::time::Duration::from_millis(t.timeout_ms), backend }
        })
```

- [ ] **Step 6: Run compiler tests**

Run: `cargo test -p nucleus-compiler`
Expected: existing tests that referenced `CompiledTest.assertion` directly now FAIL to compile. Fix each (in `nucleus-compiler` tests) to use `.assertion()` or match `body`. Re-run until PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/nucleus-compiler/src/config.rs crates/nucleus-compiler/src/solver.rs crates/nucleus-compiler/src/lib.rs
git commit -m "feat(nucleus-compiler): [[test]] type=scripted schema + TestBody enum"
```

---

### Task 6: hil `run_tests` + `assert::run` handle the `TestBody` change

**Files:**
- Modify: `crates/nucleus-hil/src/assert.rs` (`run` entrypoint at :60; reads `test.assertion`)
- Modify: `crates/nucleus-hil/src/lib.rs:88-106` (`run_tests`) + test helper `compiled_test` at :213
- Modify: `crates/nucleus-hil/tests/e2e_qemu.rs` (if it constructs/reads `CompiledTest.assertion`)

**Interfaces:**
- Consumes: `nucleus_compiler::{CompiledTest, TestBody}`.
- Produces: scripted tests become `TestStatus::Skipped` with detail `"scripted: run via `nucleus test`"` inside the in-process runner (the CLI runs them via cargo, Task 7) so `run_tests` never tries to execute a scripted body against a backend.

- [ ] **Step 1: Update `assert::run`**

Find where `run` matches on the assertion (currently `match &test.assertion { ... }` near :60). Wrap it:

```rust
pub fn run(backend: &mut dyn Backend, test: &CompiledTest) -> TestOutcome {
    let assertion = match &test.body {
        nucleus_compiler::TestBody::Declarative(a) => a,
        nucleus_compiler::TestBody::Scripted { .. } => {
            return TestOutcome {
                name: test.name.clone(),
                status: TestStatus::Skipped,
                detail: "scripted test: run via `nucleus test` (cargo)".to_string(),
            };
        }
    };
    match assertion {
        // ... existing arms unchanged, now using `assertion` instead of `&test.assertion` ...
    }
}
```

- [ ] **Step 2: Update the `compiled_test` test helper in `lib.rs`**

Change its construction from `assertion: Assertion::PinState{...}` to `body: nucleus_compiler::TestBody::Declarative(Assertion::PinState{...})`.

- [ ] **Step 3: Run hil lib tests**

Run: `cargo test -p nucleus-hil --lib`
Expected: PASS. Fix any remaining `.assertion` field references (use `.assertion()` accessor or match `body`).

- [ ] **Step 4: Fix `e2e_qemu.rs` if it references the field**

Run: `cargo test -p nucleus-hil --test e2e_qemu --no-run`
If it fails to compile on `.assertion`, update to `.assertion()` / `body`. Re-run `--no-run` until it compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/nucleus-hil/src/assert.rs crates/nucleus-hil/src/lib.rs crates/nucleus-hil/tests/e2e_qemu.rs
git commit -m "refactor(nucleus-hil): adapt assert/run_tests to CompiledTest::body"
```

---

### Task 7: CLI — run scripted tests via `cargo test`

**Files:**
- Modify: `crates/nucleus-cli/src/main.rs:379-490` (`run_test`)

**Interfaces:**
- Consumes: `nucleus_compiler::{CompiledTest, TestBody, BackendSelect}`, `std::process::Command`.
- Produces: for each scripted `CompiledTest`, runs `cargo test <script> -- --exact` in the project dir, with `NUCLEUS_TEST_BACKEND` set per selected backend, mapping exit status to PASS/FAIL. Declarative tests run through the existing backend path unchanged.

- [ ] **Step 1: Split the plan into scripted vs declarative in `run_test`**

After the `plan` is built and `test_filter` applied (around :429), partition:

```rust
    use nucleus_compiler::TestBody;
    let (scripted, declarative): (Vec<_>, Vec<_>) = plan
        .iter()
        .cloned()
        .partition(|t| matches!(t.body, TestBody::Scripted { .. }));
```

Keep the existing backend loop operating on `declarative` (rename `&plan` → `&declarative` where it calls `run_tests`).

- [ ] **Step 2: Run scripted tests via cargo**

Add, before the final exit decision:

```rust
    for test in &scripted {
        let TestBody::Scripted { script } = &test.body else { continue; };
        // Which backends to run this scripted test against.
        let backends: &[(&str, BackendArg)] = match (backend_filter, test.backend) {
            (Some(BackendArg::Qemu), _) | (None, BackendSelect::Qemu) => &[("qemu", BackendArg::Qemu)],
            (Some(BackendArg::Hardware), _) | (None, BackendSelect::Hardware) => &[("hardware", BackendArg::Hardware)],
            _ => &[("qemu", BackendArg::Qemu), ("hardware", BackendArg::Hardware)],
        };
        for (label, _) in backends {
            let status = std::process::Command::new("cargo")
                .args(["test", script, "--", "--exact", "--nocapture"])
                .current_dir(path)
                .env("NUCLEUS_TEST_BACKEND", label)
                .status();
            match status {
                Ok(s) if s.success() => println!("  PASS {} [{label}] (scripted)", test.name),
                Ok(_) => { println!("  FAIL {} [{label}] (scripted)", test.name); any_failed = true; }
                Err(e) => { eprintln!("  error: cargo test failed to launch: {e}"); any_failed = true; }
            }
        }
    }
```

(`BackendArg` is the CLI's existing backend enum — confirm its name/variants at the top of `main.rs` and match them. Adjust the `match` arms to the real `BackendSelect` mapping. The intent: a scripted test's declared `backend` plus the `--backend` flag together pick which `NUCLEUS_TEST_BACKEND` values to run.)

- [ ] **Step 3: Build the CLI**

Run: `cargo build -p nucleus-cli`
Expected: PASS. Fix borrow/move issues (the `plan` partition clones, so `plan.is_empty()` check earlier still works on the original `plan`).

- [ ] **Step 4: Manual smoke (declarative still works)**

Run: `cargo run -p nucleus-cli -- test --help`
Expected: help prints, no panic. (Full scripted run is exercised in Task 8's e2e.)

- [ ] **Step 5: Commit**

```bash
git add crates/nucleus-cli/src/main.rs
git commit -m "feat(nucleus-cli): run [[test]] type=scripted via cargo test"
```

---

### Task 8: Device test-agent firmware fixture (`agent_loopback`)

**Files:**
- Create: `crates/nucleus-hil/tests/fixtures/agent_loopback/agent.c`
- Create: `crates/nucleus-hil/tests/fixtures/agent_loopback/startup.s` (copy/adapt from `blink_itm/startup.s`)
- Create: `crates/nucleus-hil/tests/fixtures/agent_loopback/link.ld` (adapt from `blink_itm/link.ld`; add the `.nucleus_agent` section at SRAM start `0x2000_0000`)
- Create: `crates/nucleus-hil/tests/fixtures/agent_loopback/build.sh` (mirror `blink_itm/build.sh`)
- Create (committed build outputs): `agent_loopback_qemu.elf`, `agent_loopback_qemu.bin`, `agent_loopback_hw.elf`, `agent_loopback_hw.bin`

**Interfaces:**
- Produces: firmware whose RAM mailbox at `0x2000_0000` matches `protocol.rs` byte-for-byte, polling `status==BUSY`, executing commands, driving USART2 (PA2 TX / PA3 RX, 115200). Emits ITM `agent_ready` on boot.

- [ ] **Step 1: Read the blink_itm fixture for the exact toolchain/startup/linker idioms**

Read `blink_itm/startup.s`, `blink_itm/link.ld`, `blink_itm/blink_itm.c`. The agent reuses: vector table + reset handler from startup.s, RAM/flash regions from link.ld, and the ITM write helper from blink_itm.c (including the QEMU-semihosting `#ifdef HIL_QEMU_SEMIHOSTING_ITM` path and the hardware SWO setup — copy these verbatim so ITM `agent_ready` works on both).

- [ ] **Step 2: Write `link.ld` with the pinned mailbox section**

Add at the very start of the SRAM region (before `.data`/`.bss`):

```ld
  .nucleus_agent (NOLOAD) : ALIGN(4)
  {
    KEEP(*(.nucleus_agent))
  } > RAM
```

and ensure RAM `ORIGIN = 0x20000000`. In `agent.c`, place the mailbox struct in that section so it lands exactly at `0x2000_0000`:

```c
typedef struct {
    volatile uint32_t magic, version, seq, cmd, arg0, arg1, status, resp;
} mailbox_t;
__attribute__((section(".nucleus_agent"))) mailbox_t g_mbox;
```

(Linker places this struct first in the section at `0x2000_0000`. Verify with `arm-none-eabi-nm agent_loopback_hw.elf | grep g_mbox` → address `20000000`.)

- [ ] **Step 3: Write `agent.c`**

Core (constants MUST match `protocol.rs`):

```c
#include <stdint.h>

#define MAGIC        0x4E544167u
#define VERSION      1u
#define ST_IDLE 0u
#define ST_BUSY 1u
#define ST_DONE 2u
#define ST_ERR  3u
#define CMD_PING 0u
#define CMD_SET_GPIO 1u
#define CMD_READ_GPIO 2u
#define CMD_READ_REG 3u
#define CMD_UART_TX 4u
#define CMD_UART_RX_POLL 5u
#define RX_NONE 0xFFFFFFFFu

// ... RCC/GPIO/USART2 register definitions for STM32F411 ...
// Ports indexed A=0,B=1,C=2,... matching nucleus_db::Port discriminants.

static void usart2_init(void);              // PA2 AF7 TX, PA3 AF7 RX, 115200
static void itm_emit(const char *s);        // reuse blink_itm's ITM helper
static uint32_t gpio_read(uint32_t enc);    // (port<<8)|pin -> IDR bit
static void gpio_write(uint32_t enc, uint32_t level); // -> BSRR

int main(void) {
    // clocks, gpio, usart2 init
    usart2_init();
    g_mbox.magic = 0; g_mbox.version = VERSION; g_mbox.status = ST_IDLE;
    g_mbox.magic = MAGIC;     // publish last: host waits on magic
    itm_emit("agent_ready");

    for (;;) {
        if (g_mbox.status == ST_BUSY) {
            uint32_t resp = 0, err = 0;
            switch (g_mbox.cmd) {
                case CMD_PING: resp = VERSION; break;
                case CMD_SET_GPIO: gpio_write(g_mbox.arg0, g_mbox.arg1); break;
                case CMD_READ_GPIO: resp = gpio_read(g_mbox.arg0); break;
                case CMD_READ_REG: resp = *(volatile uint32_t *)g_mbox.arg0; break;
                case CMD_UART_TX: usart2_tx_byte((uint8_t)g_mbox.arg0); break;
                case CMD_UART_RX_POLL: {
                    if (usart2_rx_ready()) resp = usart2_rx_byte();
                    else resp = RX_NONE;
                } break;
                default: err = 1; break;
            }
            g_mbox.resp = resp;
            g_mbox.status = err ? ST_ERR : ST_DONE;
        }
        // also: drain USART RX into nothing here? No — UART_RX_POLL reads HW directly.
    }
}
```

USART2 RX note: the agent reads the USART data register directly on `UART_RX_POLL` (checks RXNE). A byte the host wrote to the VCP sits in the USART RDR until read; `UART_RX_POLL` returns it. Keep it simple — no ring buffer for v1.

- [ ] **Step 4: Write `build.sh`**

Mirror `blink_itm/build.sh` exactly, swapping filenames to `agent_loopback`. Keep the two builds: `_hw` (real SWO) and `_qemu` (`-DHIL_QEMU_SEMIHOSTING_ITM`).

- [ ] **Step 5: Build the fixture (requires arm-none-eabi-gcc locally)**

Run:
```bash
sh crates/nucleus-hil/tests/fixtures/agent_loopback/build.sh
arm-none-eabi-nm crates/nucleus-hil/tests/fixtures/agent_loopback/agent_loopback_hw.elf | grep g_mbox
```
Expected: build succeeds; `g_mbox` at `20000000`.

- [ ] **Step 6: Commit**

```bash
git add crates/nucleus-hil/tests/fixtures/agent_loopback
git commit -m "test(nucleus-hil): add agent_loopback device test-agent fixture"
```

---

### Task 9: e2e scripted UART loopback test (QEMU + hardware-gated)

**Files:**
- Create: `crates/nucleus-hil/tests/e2e_scripted_uart.rs`
- Modify: `crates/nucleus-hil/Cargo.toml` (`[dev-dependencies]`: add `nucleus-test-sdk = { path = "../nucleus-test-sdk" }`)
- Modify: `crates/nucleus-hil/src/qemu/mod.rs` (if needed) to expose the USART serial TCP port the test connects to — see Step 1 verification.

**Interfaces:**
- Consumes: `nucleus_test_sdk::{AgentClient, Serial}`, `nucleus_hil::qemu::QemuBackend`, `nucleus_hil::hardware::HardwareBackend`.
- The test name MUST be `uart_loopback` (matches the scripted `script = "uart_loopback"` used by the CLI in Task 7 and the docs fixture in Task 10).

- [ ] **Step 1: VERIFY QEMU USART routing FIRST (de-risk)**

Before writing the test, confirm `qemu-system-arm -M netduinoplus2` surfaces USART2 to a host socket. Manual check:
```bash
qemu-system-arm -M netduinoplus2 -nographic -serial tcp:127.0.0.1:54321,server,nowait \
  -kernel crates/nucleus-hil/tests/fixtures/agent_loopback/agent_loopback_qemu.elf &
# then connect and observe; or use -d unimp to see if usart is unimplemented
```
- If USART2 IS routed: proceed to a full two-channel test on QEMU.
- If USART2 is NOT modeled on netduinoplus2 (likely — GPIO isn't): the QEMU leg of the loopback degrades. In that case, on QEMU exercise the **mailbox half only** (`ping`, `set_gpio`/`read_gpio` via RAM, `read_register`) and assert the UART round trip on hardware only. Document the finding in the test's module doc comment (mirror how `e2e_qemu.rs` documents the GPIO gap). **Record the actual finding here before writing assertions.**

How the QEMU backend exposes the serial port: `QemuBackend::start` must launch QEMU with `-serial tcp:127.0.0.1:<port>,server,nowait` and store `<port>` so the test (or an accessor) can connect. If `start` currently uses `-serial` for something else or not at all, add the TCP serial and a `pub fn serial_port(&self) -> Option<u16>`. Keep this minimal and gated to not disturb existing e2e_qemu behavior (TIM2/ITM observation must still pass).

- [ ] **Step 2: Write the test**

```rust
//! M7 exit criterion: a scripted, host-driven UART loopback over the device
//! test-agent. TX path: host -> mailbox UART_TX -> device USART2 -> VCP -> host.
//! RX path: host -> VCP -> device USART2 RX -> mailbox UART_RX_POLL -> host.
//! Skips (doesn't fail) when the backend's tooling/board is unavailable.

use std::time::Duration;
use nucleus_hil::backend::{Backend, FirmwareArtifact};
use nucleus_test_sdk::{AgentClient, Serial};

fn backend_env() -> String {
    std::env::var("NUCLEUS_TEST_BACKEND").unwrap_or_else(|_| "qemu".to_string())
}

#[test]
fn uart_loopback() {
    // Dispatch on NUCLEUS_TEST_BACKEND so `nucleus test` (Task 7) can target
    // each backend; default qemu for a bare `cargo test`.
    match backend_env().as_str() {
        "hardware" => run_hardware(),
        _ => run_qemu(),
    }
}

fn run_qemu() {
    if std::process::Command::new("qemu-system-arm").arg("--version").output().is_err() {
        eprintln!("skipping: qemu-system-arm not installed");
        return;
    }
    // ... start QemuBackend with the agent_loopback_qemu.elf, connect AgentClient,
    // assert ping()==1, set/read GPIO via mailbox; if USART routed (Step 1),
    // open Serial::open_tcp(qemu serial port) and assert the TX/RX round trips
    // within 10ms; otherwise assert mailbox half only and note the gap.
}

fn run_hardware() {
    // Gate like e2e_hardware_replay.rs: skip if no board/openocd/st-flash.
    // start HardwareBackend with agent_loopback_hw.bin/elf, connect AgentClient,
    // ping()==1, then:
    //   TX: agent.uart_tx(b); assert Serial::open_device("/dev/ttyACM0",115200)
    //       reads b within 10ms.
    //   RX: serial.write_byte(b); assert agent.uart_rx_poll() == Some(b) within 10ms.
}
```

Fill `run_qemu`/`run_hardware` bodies concretely once Step 1's finding is known. Each round trip wrapped in a `let t0 = Instant::now(); ...; assert!(t0.elapsed() < Duration::from_millis(10))`.

- [ ] **Step 3: Run on QEMU**

Run: `cargo test -p nucleus-hil --test e2e_scripted_uart`
Expected: PASS (full loopback if USART routed; mailbox-half + documented gap otherwise). SKIP message if QEMU absent.

- [ ] **Step 4: Commit**

```bash
git add crates/nucleus-hil/tests/e2e_scripted_uart.rs crates/nucleus-hil/Cargo.toml crates/nucleus-hil/src/qemu/mod.rs
git commit -m "test(nucleus-hil): e2e scripted UART loopback (M7)"
```

---

### Task 10: Wire the scripted fixture into a `[[test]]` + docs + live hardware + issue update

**Files:**
- Create: `crates/nucleus-hil/tests/fixtures/agent_loopback/stm32.toml` (a project config with `[[test]] type="scripted" script="uart_loopback"`)
- Create: `docs/src/m7-scripted-tests.md` (protocol + SDK usage)
- Modify: `docs/src/SUMMARY.md` (link the new page — follow existing mdbook structure)
- Modify: GitHub issue #21 body (tick M7 boxes) + post a comment

**Interfaces:** none (integration + docs).

- [ ] **Step 1: Author the scripted `[[test]]` config**

`stm32.toml` (minimal valid F411RE config with USART2 + the scripted test):

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

Verify it validates: `cargo run -p nucleus-cli -- check crates/nucleus-hil/tests/fixtures/agent_loopback` → exit 0.

- [ ] **Step 2: Write the docs page**

`docs/src/m7-scripted-tests.md` covering:
- When to use scripted vs declarative (M6) tests.
- Mailbox protocol v1: address `0x2000_0000`, field table, status machine, command table — copied from `protocol.rs` and `agent.c` (single source of truth note).
- Versioning rule: `version` field; SDK rejects mismatch; bump on incompatible changes.
- `nucleus_test_sdk` usage: `AgentClient` + `Serial`, a short loopback code sample.
- Port index mapping (A=0, B=1, …) used by `encode_pin`.

- [ ] **Step 3: Docs build**

Run: `cd docs && mdbook build` (or the repo's documented docs command)
Expected: builds, new page linked.

- [ ] **Step 4: Full check suite**

Run: `make check`
Expected: fmt-check + clippy(-D warnings) + tests all PASS.

- [ ] **Step 5: LIVE HARDWARE verification (board connected)**

With a NUCLEO-F411RE connected, reap stray openocd first:
```bash
pkill openocd || true
NUCLEUS_TEST_BACKEND=hardware cargo test -p nucleus-hil --test e2e_scripted_uart uart_loopback -- --exact --nocapture
```
Expected: PASS — TX byte observed on `/dev/ttyACM*`, RX byte observed via `uart_rx_poll`, both ≤10ms. If SWO/RSP flakiness recurs, note it honestly in the commit/issue (per M6 precedent) — do NOT mark the box done if it doesn't pass live.

- [ ] **Step 6: Commit**

```bash
git add crates/nucleus-hil/tests/fixtures/agent_loopback/stm32.toml docs/src/m7-scripted-tests.md docs/src/SUMMARY.md
git commit -m "docs(M7): scripted-tests guide + scripted [[test]] fixture config"
```

- [ ] **Step 7: Update issue #21**

Tick the five M7 acceptance boxes in the issue **body** (per the repo's GitHub workflow), and post a comment summarizing: what landed, the QEMU USART finding from Task 9 Step 1, and the live-hardware result (honest pass/flaky note).

---

## Self-Review

**Spec coverage:**
- Device test-agent spec → Tasks 8 (firmware), 3 (protocol consts), 10 (docs). ✓
- Host SDK `nucleus_test_sdk` → Tasks 3 (agent), 4 (serial). ✓
- Example fixture UART loopback both backends → Tasks 8, 9, 10. ✓
- M6 integration `type=scripted` → Tasks 5 (schema/compiler), 6 (hil), 7 (CLI). ✓
- Documentation → Task 10. ✓
- Backend `write` half (design §2) → Tasks 1, 2. ✓

**Placeholder scan:** Task 9 deliberately defers concrete `run_qemu`/`run_hardware` bodies to the QEMU-USART finding in Step 1 — this is a real verification gate, not a placeholder; the structure, gating, and assertions (10ms windows, skip conditions) are specified. All other steps carry complete code.

**Type consistency:** `read_mem32`/`write_mem32` (Tasks 1,2,3) consistent. `TestBody::{Declarative,Scripted}` + `CompiledTest::body` + `.assertion()` consistent across Tasks 5,6,7. `AgentClient`/`SdkError`/`Serial` names consistent across Tasks 3,4,9. Protocol consts (Task 3) ↔ firmware `#define`s (Task 8) flagged as must-match.

**Known cross-task risk:** `nucleus_db::Port as u32` discriminant ordering must equal the agent's port indexing — flagged in Tasks 3 (Step 2) and 8 (Step 3) and documented in Task 10.
