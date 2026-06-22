# M10 — Lockstep Co-Execution (The Crown)

Status: approved design, pre-implementation.
Tracks: GitHub issue #21, milestone M10.

## Purpose

Run the same firmware on the QEMU twin and real silicon, compare observable
state at sync points nucleus already controls, and report the first point
where the two backends disagree.

Scope decided with the user up front:

- **No explanation engine.** The report is a bare diff: checkpoint index,
  diverging observable, sim value, silicon value. No clock-skew/erratum
  inference, no knowledge base. Deferred to v3.
- **Sync points are ITM events; observables are `[trace.variables]`.** No new
  TOML schema (no `[[lockstep]]` block). Every ITM event the firmware emits
  is a checkpoint; at each checkpoint, snapshot every register already named
  in `[trace.variables]`. Reuses the M9 trace config as-is.
- **Fixtures are synthetic + QEMU e2e, not live two-backend divergence.**
  QEMU's `netduinoplus2` machine has no GPIO model and real hardware SWO/GDB
  capture is known-flaky (see M5/M7 findings). A deterministic CI fixture
  cannot depend on a live QEMU-vs-silicon mismatch happening on demand.

## Architecture

New module: `nucleus-hil::lockstep`.

Two phases, collect then compare:

### Collect

**Correction (made during implementation planning):** `[trace.variables]`
entries (`nucleus_compiler::config::TraceVariable { name, port, ty }`)
carry no memory address — the firmware writes the variable's value
directly onto an ITM stimulus port, there is no SWD register read
involved. So a checkpoint's "snapshot" isn't a `Backend::register()` /
`Backend::read_mem32()` call; the value already arrives inside the ITM
event itself.

For each backend, drive the same event loop used by `assert::run`'s ITM
path: call `Backend::await_itm_event(timeout)` in a loop until it returns
`None` (timeout, no more events) or a run-level timeout elapses. On every
`Some(event)`, decode it via `nucleus_trace::translate::{Translator,
VariableMap}` — the same machinery the trace daemon uses — by replaying it
as a `nucleus_itm::Packet::Instrumentation { port, data }` (the same shape
`ItmEvent` already carries). A configured port decodes to one
`TraceEvent::Variable { name, value, .. }`; port 0 (the log stream) and any
unconfigured port decode to nothing, and the checkpoint still compares on
its raw event. Append one `Checkpoint` per ITM event observed.

Output: one `ObservationTrace` per backend, collected independently (each
backend runs to completion/timeout on its own — this is not real-time
lockstep, it's post-hoc comparison of two completed runs, consistent with
the issue's "QEMU is not cycle-accurate" framing).

### Compare

Walk both traces by index. At each index:

1. If one trace has run out of checkpoints and the other hasn't, that index
   is the divergence (`observable = "itm_event"`, the missing side's value
   is `"<no event>"`).
2. If the ITM event payloads differ, that index is the divergence
   (`observable = "itm_event"`).
3. Otherwise, compare every named snapshot value at this checkpoint. First
   mismatching name is the divergence (`observable = <var name>`).

First mismatch found (by any of the three rules) short-circuits the walk.
No mismatch through the shorter trace's length (with equal lengths) means
agreement end-to-end.

## Data shapes

In `nucleus-hil::lockstep`:

```rust
pub struct Checkpoint {
    pub itm_event: ItmEvent,
    pub decoded: Option<(String, serde_json::Value)>, // (variable name, decoded value), if this port is configured
}

pub struct ObservationTrace {
    pub checkpoints: Vec<Checkpoint>,
}

pub enum DivergenceReport {
    Agreement {
        checkpoints_compared: usize,
    },
    Diverged {
        first_checkpoint: usize,
        observable: String,   // variable name, or "itm_event"
        sim_value: String,
        silicon_value: String,
    },
}
```

`DivergenceReport` is the "certificate" the issue describes: `Agreement`
is the proof of end-to-end match; `Diverged` is the located, bare-diff
disagreement.

## CLI

New verb: `nucleus lockstep [path]`

- Reuses the same firmware artifact lookup as `nucleus test`
  (`build/firmware` + `build/firmware.bin`; errors the same way if missing).
- Runs collect on `QemuBackend` and `HardwareBackend` (same construction
  pattern as `run_test`'s backend list).
- Per-backend tool/hardware unavailability degrades to a skipped leg, not a
  failure — mirrors `nucleus test`'s `HilError::ToolMissing` handling. If a
  leg is skipped, lockstep cannot compare; prints "one leg only, no
  comparison possible" and exits success (nothing to disagree about).
- If both legs run: prints the `DivergenceReport` (human-readable: either
  "agreement across N checkpoints" or "diverged at checkpoint K: <var> sim=X
  silicon=Y"). Exit non-zero only on `Diverged`.
- `--explain` flag: accepted for forward CLI compatibility with the issue's
  acceptance text, but is a no-op that prints "explanation not implemented
  in this release — see v3" rather than a bare diff repeated. Keeps the
  flag's existence honest without building unrequested inference logic.

## Testing

- **Comparator unit tests**: hand-built `ObservationTrace` pairs.
  - Two identical traces → `Agreement`.
  - Two traces differing in one snapshot value at checkpoint 2 →
    `Diverged { first_checkpoint: 2, .. }` with correct observable name and
    both values.
  - Two traces differing in ITM event payload → `Diverged` with
    `observable == "itm_event"`.
  - One trace shorter than the other → `Diverged` at the shorter trace's
    length, `observable == "itm_event"`, missing side reported as
    `"<no event>"`.
- **QEMU e2e**: one real `qemu-system-arm` run collected via `collect()`,
  compared against itself (or a second independent run of the same
  firmware) to prove the agreement-cert path works against a real ITM
  stream end-to-end, following the existing `e2e_qemu.rs` pattern from M6.
- **Hardware leg**: best-effort, like M5/M7 — skips cleanly if no board is
  attached. No live two-backend divergence fixture; that would require a
  real mismatch on demand, which is not deterministic enough for CI.

## Out of scope (explicitly deferred)

- Explanation engine (clock-skew detection, erratum lookup, emulator-gap
  knowledge base) — v3.
- New `[[lockstep]]` TOML schema for declaring custom sync points or
  observable sets beyond `[trace.variables]` — v3, if ITM+trace-variables
  coverage proves insufficient in practice.
- Real-time/streaming lockstep (comparing both backends as they run rather
  than after each completes) — not attempted; QEMU's non-cycle-accuracy
  makes this unnecessary per the issue's own framing.
- Demo GIF — maintainer step, not part of the engineering deliverable.
