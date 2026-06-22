# Dual-Backend HIL Substrate

Nucleus v1 stopped at "does this firmware build and flash." v2's whole second
half is "does this firmware actually *behave* correctly" — and answering that
needs a way to observe a running target. The M5 substrate, `nucleus-hil`,
gives every later test feature (declarative assertions, scripted tests,
lockstep) one common `Backend` trait so they never care whether the target is
a simulator or real silicon.

## The `Backend` trait (`nucleus-hil/src/backend.rs`)

```rust
pub trait Backend {
    fn name(&self) -> BackendKind;
    fn start(&mut self, firmware: &FirmwareArtifact, check_report: &CheckReport)
        -> Result<(), HilError>;
    fn pin(&mut self, port: nucleus_db::Port, pin_num: u8) -> Result<bool, HilError>;
    fn register(&mut self, peripheral: &str, offset: u32) -> Result<u32, HilError>;
    fn read_mem32(&mut self, addr: u32) -> Result<u32, HilError>;
    fn write_mem32(&mut self, addr: u32, value: u32) -> Result<(), HilError>;
    fn await_itm_event(&mut self, timeout: Duration) -> Result<Option<ItmEvent>, HilError>;
    fn finish(&mut self) -> Result<(), HilError>;
}
```

Every test feature built on top — `[[test]]` assertions (M6), scripted tests
(M7), lockstep (M10) — only ever calls these seven methods. A new backend
(a different simulator, a different probe) only has to implement this trait
once to plug into everything that already exists.

Key supporting types:

- `BackendKind` — `Qemu` | `Hardware`.
- `FirmwareArtifact { elf, bin }` — the build output `nucleus build` produces.
- `ItmEvent { port, data }` — one decoded ITM stimulus-port packet.
- `SampleTarget` — `Pin { port, pin_num }` or `RegisterChanged { peripheral, offset }`, what lockstep's checkpoints sample.
- `HilError` — `Preflight` | `ToolMissing` | `Io` | `Protocol` | `NotObservable`. `ToolMissing` (e.g. no `qemu-system-arm` on `PATH`, no probe attached) is treated as **skip**, not **fail** — see below.
- `RunStatus` — `Completed` | `Skipped { reason }` | `Failed { error }`.

## Two backends, one fidelity gap

### QEMU backend

Runs firmware under `qemu-system-arm -M netduinoplus2` (the closest QEMU
machine to an F4 Nucleo board). No probe, no hardware needed — always
available in CI.

**Empirically confirmed fidelity gap:** this QEMU machine has **no real GPIO
model**. `pin()` reads/writes hit an `unimplemented_device` stub — writes are
silently dropped, reads always return `0`. It **does** fully model USART2
bidirectionally via `serial_hd(1)`, so UART-based tests work end-to-end on
QEMU. This is why M7's scripted UART loopback test passes identically on
both backends, while a GPIO toggle assertion is only meaningfully verified on
hardware — `nucleus test`'s declarative engine still runs the GPIO assertion
on QEMU (it doesn't special-case this), but it can only ever observe `0`/no
toggling there.

### Hardware backend

Drives a real attached Nucleo board over SWD/SWO (via OpenOCD), reusing
`nucleus-trace`'s ITM decode path (`nucleus-itm`) for `await_itm_event`. If
no board/probe is present, `start()` returns `HilError::ToolMissing` and
`nucleus test` reports that backend as **skipped**, not failed — a CI runner
with no hardware attached still gets a green run for the backends it *can*
exercise, never a false failure for the ones it can't.

## Running tests: `nucleus test`

```sh
nucleus test [path]                  # both backends, every [[test]] block
nucleus test [path] --backend qemu   # one backend only
nucleus test [path] --backend hardware
nucleus test [path] --test <name>    # one test only
```

`path` defaults to `.` (expects `stm32.toml` and `build/firmware.{elf,bin}` —
run `nucleus build` first). For each declarative test, on each selected
backend: `start()`, evaluate the assertion (see [Declarative
Tests](declarative-tests.md)), `finish()`. Each result prints as
`PASS`/`FAIL`/`SKIP` and is appended to `tests/test_history.json` (see [Test
History](test-history.md)). Exit code `1` if any test fails on any backend
that actually ran (skips never fail the run).

## See also

- [Declarative Tests](declarative-tests.md) — the `[[test]]` assertion grammar this substrate evaluates
- [Scripted Tests](scripted-tests.md) — the device-agent SDK built on the same `Backend` trait
- [Lockstep Co-Execution](lockstep.md) — runs both backends side-by-side over this same trait to detect divergence
- [Test History & CI](test-history.md) — where `nucleus test`'s results end up
