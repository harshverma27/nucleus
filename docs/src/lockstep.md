# Lockstep Co-Execution

The crown of v2: run the **same firmware** on both HIL backends at once and
detect the moment they disagree. Passing on QEMU only proves your logic is
correct in an idealized model — lockstep is how you find out a timing
assumption, a register quirk, or a peripheral behavior QEMU doesn't model
faithfully is masking a real bug that only shows up on silicon.

## How it works (`nucleus-hil/src/lockstep.rs`)

Sync points are **ITM events** — both backends already expose
`Backend::await_itm_event`, so no new backend capability is needed.
Observables are whatever `[trace.variables]` decodes a port's bytes into
(the same `Translator` the trace daemon and dashboard use): a traced
variable's value already arrives inside the ITM packet that names its port.

1. **Collect** — `collect(backend, vars, timeout_per_event, total_timeout)`
   drives one already-started backend with `await_itm_event` until it times
   out or stops producing events, decoding each event into a `Checkpoint {
   itm_event, decoded }`. `decoded` is `None` for port-0 log lines and
   unconfigured ports — they still compare on the raw event bytes.
2. **Compare** — `compare(sim_trace, silicon_trace)` walks both
   `ObservationTrace`s checkpoint by checkpoint and returns a
   `DivergenceReport`:
   - `Agreement { checkpoints_compared }` — same length, same content at
     every index.
   - `Diverged { first_checkpoint, observable, sim_value, silicon_value }` —
     the first index where they disagree. A differing **decoded** value is
     reported by variable name (friendlier); a differing **raw** event, or
     one trace running out before the other, is reported as `"itm_event"`.

This is a **bare diff, by design** — no inference, no root-cause engine.
Explaining *why* two traces diverged (was it a timing race? a register model
gap? a real bug?) is explicitly deferred past v2; `nucleus lockstep
--explain` exists as a flag but only prints a note that it isn't implemented
yet, never silently swallowed.

## Running it: `nucleus lockstep`

```sh
nucleus lockstep [path]            # both backends, compare
nucleus lockstep [path] --explain  # same, plus an "not implemented, see v3" note on divergence
```

`path` defaults to `.` (expects `stm32.toml` and `build/firmware.{elf,bin}`
— run `nucleus build` first). Sequence: run `nucleus check` (abort on any
conflict, same as `nucleus test`) → start both backends (a `ToolMissing`
backend — e.g. no QEMU binary, no probe attached — is reported as
**skipped**, not a hard failure) → `collect` from each → `compare` the two
that started.

If fewer than two backends actually started, lockstep prints which leg (if
any) ran and exits `0` — "no comparison possible" is not itself a failure.
With two legs:

```sh
$ nucleus lockstep
agreement across 12 checkpoint(s) (Qemu vs Hardware)
```

```sh
$ nucleus lockstep
diverged at checkpoint 4: speed Qemu=10 Hardware=12
```

Exit code: `0` on agreement (or fewer than two legs), `1` on a detected
divergence — the gating signal for "trust this build before flashing it."

## Why this catches what single-backend testing can't

Recall the [QEMU fidelity gap](hil-backends.md#two-backends-one-fidelity-gap):
the `netduinoplus2` machine fully models USART2 but has no real GPIO model.
A test that only runs on QEMU can pass cleanly while quietly never
exercising real GPIO behavior at all. Lockstep doesn't fix the simulator's
fidelity gap — it makes that gap **visible**: a divergence at a
GPIO-driven checkpoint is the system telling you "QEMU and hardware
disagree here," which is exactly the signal a model gap (or a real bug)
produces.

## See also

- [Dual-Backend HIL Substrate](hil-backends.md) — the `Backend` trait and the fidelity gap this builds on
- [Declarative Tests](declarative-tests.md), [Scripted Tests](scripted-tests.md)
- [Trace Dashboard & ITM Decoding](itm-trace.md) — the same `Translator`/`[trace.variables]` config lockstep decodes checkpoints with
