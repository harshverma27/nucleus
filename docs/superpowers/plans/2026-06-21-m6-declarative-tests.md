# M6 — Declarative Tests

## Context

M5 (`e297205`–`57cdeb5`) shipped the dual-backend HIL substrate: `nucleus-hil`'s `Backend` trait (`QemuBackend`, `HardwareBackend`) gives both backends a shared observation API — `pin()`, `register()`, `await_itm_event()`, `sample()` — but, per `backend.rs`'s own doc comment, "every method here is observation, not assertion — `[[test]]` assertions are M6." M6 is the next item on Week 2 (issue #21, branch `21-v2-week-2-tests-ledger-lockstep-crown`): turn that observation API into the declarative test surface — `[[test]]` blocks in `stm32.toml`, parsed/validated/compiled like every other section, executed by a new runner in `nucleus-hil`, exposed via a new `nucleus test` CLI verb.

Per CLAUDE.md's GitHub workflow: post this plan as a comment on issue #21 before starting, push to `21-v2-week-2-tests-ledger-lockstep-crown`, and tick the M6 checklist items in the issue body (not a comment) as they land.

## Design decisions

- **Schema mirrors `[[exti]]`/`[[trace.variables]]` exactly**: a new `Vec<TestCase>` field on `Config`, `#[serde(deny_unknown_fields)]`, no parser changes beyond adding the struct — same zero-new-parsing-machinery pattern M1–M3 followed.
  ```toml
  [[test]]
  name = "uart2_echo"
  assertion = "UART2 echoes 'ping' within 10ms"
  timeout_ms = 100        # optional, default 1000
  backend = "both"        # optional: "qemu" | "hardware" | "both" (default)
  ```
- **Assertion strings are parsed, not freeform.** A small hand-written recursive-descent parser (no new dependency — `nucleus-compiler` already hand-parses nothing like this, but the grammar is tiny: subject + verb + object + optional qualifiers) turns `"pin PA5 toggles at 1Hz ±5%"` into a typed `Assertion` enum. Three variants for M6's vocabulary:
  - `Assertion::PinToggles { pin: Pin, hz: f64, tolerance_pct: f64 }`
  - `Assertion::PinState { pin: Pin, level: bool, within: Duration }`
  - `Assertion::UartEcho { instance: String, payload: Vec<u8>, within: Duration }`
  - `Assertion::ItmEvent { pattern: String, within: Duration }`
  Each maps 1:1 onto one `Backend` method already in `backend.rs` (`PinToggles`/`PinState` → `sample`/`pin`, `UartEcho` → TX via test-agent stub *or* deferred to M7 — see below, `ItmEvent` → `await_itm_event`). I2C ACK detection from the issue's vocabulary is listed but has no existing `Backend` primitive (no `register`-level I2C status read modeled) — **deferred to M7's scripted-test escape hatch**, documented as a known gap rather than half-implemented here.
- **`UartEcho` is the one assertion that needs to *drive* the device, not just observe it** — `Backend` today is observation-only. Two options: (a) extend `Backend` with a `send_uart` method now, or (b) scope M6's UART assertion to a passive form (firmware echoes on boot, test only observes the echo via ITM) and push interactive stimulus to M7's host SDK, which is explicitly designed for "drive input, read output" sequences. **Choosing (b)**: M6's `UartEcho` assertion observes via `await_itm_event` (firmware logs the received byte over ITM after echoing), with the literal interactive "host sends byte over UART" form deferred to M7. This keeps M6 inside the existing `Backend` trait — zero trait changes — and matches the issue's own dependency note that M7 is "the power-user escape hatch... beyond pin toggle / ITM events."
- **Validator runs at parse time, in `nucleus-compiler`, not at run time in `nucleus-hil`.** Same layering as M1–M3: `nucleus-compiler` knows the family's pins/peripherals and can reject `pin PZ99` or `UART9 echoes` (unknown peripheral) before anything boots. Reuses `Conflict`'s existing severity/Display/LSP machinery — new variant `Conflict::InvalidTest { name: String, reason: String }`.
- **Compiler emits a parsed `TestPlan`, not text.** `nucleus_compiler::test_plan(config: &Config) -> Result<Vec<CompiledTest>, Vec<Conflict>>` where `CompiledTest { name: String, assertion: Assertion, timeout: Duration, backend: BackendSelect }` — this is the "parsed form `nucleus-hil` consumes" the issue specifies. `nucleus-hil` never touches TOML or `Config` for assertions, only this typed plan — keeps the parsing/validation and execution layers as separated as M1–M3's model/solver split.
- **Runner lives in `nucleus-hil` as a new `assert.rs` module**, `pub fn run(backend: &mut dyn Backend, test: &CompiledTest) -> TestOutcome` where `TestOutcome { name: String, status: TestStatus, detail: String }` (`TestStatus::Passed | Failed | Skipped`, mirroring `RunStatus`'s `Skipped`-is-not-`Failed` distinction from M5). Each `Assertion` variant maps to one backend call:
  - `PinToggles` → `backend.sample(window)`, count rising edges in `readings`, compare measured Hz against `hz ± tolerance_pct`.
  - `PinState` → `backend.pin(port, pin_num)` polled until `level` matches or `within` elapses.
  - `ItmEvent`/`UartEcho` → `backend.await_itm_event(timeout)`, match `data` against `pattern`/`payload` (substring match for ITM logs, since the issue's example is a string prefix match: `"trace event \"boot_done\""`).
- **`nucleus test` is a new CLI verb**, not a flag on `build`, consistent with M4's "one verb per milestone" precedent (`route`, soon `test`/`history`/`show`/`lockstep`). `Command::Test { path: PathBuf, backend: Option<BackendFilter>, test: Option<String> }` — `--backend qemu|hardware` selects one (default both), `--test <name>` runs one (default all). Exit non-zero if any selected test fails on any selected backend; `Skipped` (tool/hardware unavailable) does not fail the run, matching M5's `RunStatus::Skipped` semantics.
- **Fixture suite reuses `crates/nucleus-hil/tests/`'s existing QEMU/hardware-replay split** (`e2e_qemu.rs`, `e2e_hardware_replay.rs`) rather than inventing a third test-running convention. Add `[[test]]` blocks to whatever fixture firmware those files already boot.

## Files

### 1. `crates/nucleus-compiler/src/config.rs`
Add `pub test: Vec<TestCase>` field to `Config` (mirrors `exti: Vec<ExtiPin>`). New struct:
```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, rename = "test")]
pub struct TestCase {
    pub name: String,
    pub assertion: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub backend: Option<String>, // "qemu" | "hardware" | "both"; None = both
}
```
Unit tests alongside existing `config.rs` tests: parses a minimal `[[test]]` block; rejects unknown field; defaults `timeout_ms`.

### 2. `crates/nucleus-compiler/src/assertion.rs` (new)
The assertion grammar + `Assertion` enum + hand-written parser (`fn parse(s: &str) -> Result<Assertion, String>`). Grammar subset for M6: `pin <PIN> toggles at <N>Hz ±<N>%`, `pin <PIN> is <high|low> within <N>ms`, `<PERIPH> echoes "<text>" within <N>ms`, `trace event "<pattern>" within <N>ms`. Unit tests: one per grammar form, plus malformed-string rejection cases (unknown verb, missing unit, negative duration).

### 3. `crates/nucleus-compiler/src/solver.rs`
Add `Conflict::InvalidTest { name: String, reason: String }` (11th variant after M4's `Unroutable`). `Display`/`severity()` arms follow the `Unroutable` pattern exactly (always `Severity::Error`). Wire into `solve()`: for each `config.test` entry, parse the assertion (via `assertion::parse`) and validate referenced pins/peripherals exist in `db` for the resolved family — same `find_af`/peripheral-lookup calls M1–M3 already use, reused, not duplicated.

### 4. `crates/nucleus-compiler/src/lib.rs`
Add `pub mod assertion;` and `pub use assertion::Assertion;`. Add:
```rust
pub struct CompiledTest {
    pub name: String,
    pub assertion: Assertion,
    pub timeout: Duration,
    pub backend: BackendSelect, // enum Qemu | Hardware | Both
}
pub fn test_plan(config: &Config) -> Result<Vec<CompiledTest>, Vec<Conflict>>
```
`test_plan` reuses `solver::solve`'s validation (so a config with an invalid test never silently produces a plan) and only emits the compiled list when `conflicts.is_empty()`.

### 5. `crates/nucleus-hil/src/assert.rs` (new)
`pub fn run(backend: &mut dyn Backend, test: &nucleus_compiler::CompiledTest) -> TestOutcome`. One match arm per `Assertion` variant, each calling exactly one `Backend` method as described in Design decisions. `TestOutcome`/`TestStatus` defined here (re-exported from `lib.rs`), deliberately separate from M5's `RunResult`/`RunStatus` since a `RunResult` is backend-lifecycle-scoped and a `TestOutcome` is per-assertion. Unit tests with a fake in-module `Backend` impl (same pattern M5 likely used for `backend.rs`'s own tests — check `qemu/mod.rs`/`hardware/mod.rs` for an existing test-double pattern to reuse rather than inventing a new one).

### 6. `crates/nucleus-hil/src/lib.rs`
Add `pub mod assert;` and a top-level `pub fn run_tests(backend: &mut dyn Backend, plan: &[CompiledTest]) -> Vec<TestOutcome>` that loops `assert::run` per test, filtering by `CompiledTest::backend` against `backend.name()` (skip with `TestStatus::Skipped` if the test doesn't target this backend) — this is what `nucleus-cli`'s `run_test` calls once per backend.

### 7. `crates/nucleus-cli/src/main.rs`
Add `Command::Test { path: PathBuf, backend: Option<String>, test: Option<String> }` (clap, mirrors `Check`'s `path` arg plus `Trace`'s flag style) and `run_test()`: read file → `nucleus_compiler::test_plan` → for each selected backend (`QemuBackend`/`HardwareBackend`, instantiated the same way M5's e2e tests do) → `nucleus_hil::run_tests` → print one line per `TestOutcome` (pass/fail/skip, name, backend) → `ExitCode::FAILURE` if any `Failed`, else `SUCCESS`. `--test <name>` filters `plan` to one entry before running.

### 8. `crates/nucleus-lsp/src/analysis.rs`
Add the `Conflict::InvalidTest` arm to `conflict_spans` (exhaustive match, same mechanical requirement M4 hit with `Unroutable`) — locate the `[[test]]` table by `name` field via text search, same fallback style already used for array-of-table entries (`exti`/`trace.variables` — check how those resolve spans today and mirror it). Add hover for the `assertion` key (show the parsed grammar forms) and completion for `backend = ` values (`"qemu"`, `"hardware"`, `"both"`) in `completion()`, alongside the existing pin-role completions.

### 9. Fixtures + tests
New `[[test]]` blocks added to the fixture configs `crates/nucleus-hil/tests/e2e_qemu.rs` and `e2e_hardware_replay.rs` already boot (check what firmware/config those reference first — reuse, don't fork). One simple fixture (single `pin ... toggles` or `trace event` test) and one complex fixture (multiple assertion kinds) per the issue's acceptance criteria, run against both backends with the *same* expected result. New CLI integration test in `crates/nucleus-cli/tests/cli.rs`, a `run_test(name)` helper mirroring `run_check`/`run_route`.

## Known gaps (documented, not silently dropped)

- **I2C ACK detection** (`I2C1 device 0x68 ACKs`): no `Backend` primitive exists for it. Deferred to M7 (scripted tests can read I2C status registers via `backend.register()` once the test-agent protocol defines the right offset/op).
- **Interactive UART stimulus** (host sends a byte, not just observes an echo firmware already triggers): deferred to M7's host SDK, which is built exactly for stimulus/response sequences `Backend`'s observation-only trait can't do alone.
