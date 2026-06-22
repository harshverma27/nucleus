# M10 Lockstep Co-Execution — Implementation Plan

**Goal:** Run firmware on both HIL backends, collect ITM-event checkpoints
from each, and report the first point where they disagree.

**Status:** implemented (this plan documents what was built, written
alongside the implementation per this session's workflow rather than
strictly before it).

**Tracks:** GitHub issue #21, milestone M10. Design spec:
`docs/superpowers/specs/2026-06-23-m10-lockstep-design.md` (note: that spec
was corrected during this implementation — see below).

## Scope (locked with the user before implementation)

- Bare diff only, no explanation engine (deferred to v3).
- Sync points = ITM events; observables = `[trace.variables]`.
- Fixtures: synthetic `ObservationTrace` unit tests + one real QEMU e2e
  agreement-path test. No live two-backend divergence fixture (QEMU has no
  GPIO model on `netduinoplus2`; hardware SWO/GDB is known-flaky per
  M5/M7).

## Correction found during planning

The posted design spec originally said checkpoint snapshots come from
`Backend::register()` / `Backend::read_mem32()` calls keyed by
`[trace.variables]`. That's wrong: `TraceVariable { name, port, ty }`
carries no address — firmware writes the value directly onto an ITM
stimulus port. The value already arrives inside the ITM event; decoding
uses `nucleus_trace::translate::{Translator, VariableMap}` against a
`nucleus_itm::Packet::Instrumentation { port, data }` built from the
already-captured `ItmEvent`. This is strictly simpler than the original
spec (no extra backend round-trip per checkpoint) and is what's
implemented. The spec doc was amended in place to match.

## What was built

**`crates/nucleus-hil/src/lockstep.rs`** (new):
- `Checkpoint { itm_event: ItmEvent, decoded: Option<(String, serde_json::Value)> }`
- `ObservationTrace { checkpoints: Vec<Checkpoint> }`
- `DivergenceReport` — `Agreement { checkpoints_compared: usize }` or
  `Diverged { first_checkpoint, observable, sim_value, silicon_value }`
- `collect(backend, vars, timeout_per_event, total_timeout) -> ObservationTrace`
  — loops `Backend::await_itm_event` until `Ok(None)` or `total_timeout`
  elapses, decoding each event via `Translator`.
- `compare(sim, silicon) -> DivergenceReport` — walks both traces by index;
  decoded-value mismatch is checked before raw-event mismatch (it's the
  friendlier observable name, and a differing decoded value always implies
  differing raw bytes, so checking decoded first is what actually surfaces
  variable names instead of opaque byte dumps); unequal lengths report
  `"<no event>"` on the shorter side.
- 6 unit tests against synthetic `ObservationTrace` pairs (agreement,
  decoded-value divergence, raw-event divergence, length-mismatch
  divergence, plus two `decode()` unit tests).

**`crates/nucleus-hil/src/lib.rs`** — `pub mod lockstep;` +
re-export of `Checkpoint, ObservationTrace, DivergenceReport`.

**`crates/nucleus-hil/Cargo.toml`** — added `serde_json = "1"` (needed for
`Value` in `Checkpoint::decoded`; `nucleus-trace` and `nucleus-itm` were
already dependencies).

**`crates/nucleus-cli/src/main.rs`** — `Command::Lockstep { path, explain }`,
dispatched to `run_lockstep()`. Mirrors `run_test`'s firmware-artifact
lookup and conflict-reporting; always runs both backends (no `--backend`
filter, since lockstep's entire point is comparing both); a
`HilError::ToolMissing` leg is skipped and printed, not failed; if fewer
than 2 legs ran, prints "one leg only — no comparison possible" and exits
success; otherwise runs `lockstep::compare` and prints the human-readable
report, exiting non-zero only on `Diverged`. `--explain` is parsed and
prints "not implemented in this release — see v3" after a divergence,
never changes exit behavior.

**`crates/nucleus-hil/tests/e2e_lockstep_qemu.rs`** (new) — two tests
against real `qemu-system-arm` (skips cleanly if not installed):
`collect_captures_real_qemu_itm_boot_log` proves the collect loop captures
real ITM events into checkpoints; `comparing_two_collections_of_the_same_firmware_agrees`
proves two independent collections of the same firmware reach `Agreement`
(the certificate path).

## Verification performed

- `cargo test -p nucleus-hil lockstep` — 6/6 unit tests pass.
- `cargo test -p nucleus-hil --test e2e_lockstep_qemu` — 2/2 pass against
  real `qemu-system-arm`.
- `cargo test -p nucleus-hil` (full suite) — 52/53 pass; the one failure
  (`hardware::tests::flashes_and_observes_a_real_board_when_present`) is
  pre-existing and unrelated (confirmed by stashing all lockstep changes
  and re-running it — same failure on the unmodified tree, this
  environment has no board attached).
- `cargo test -p nucleus-cli` — all pass.
- `cargo build -p nucleus-cli` and `nucleus --help` — `lockstep` verb
  listed correctly.
- `cargo clippy -p nucleus-hil -p nucleus-cli -- -D warnings` — clean.

## Not done (explicitly out of scope, per locked decisions above)

- Explanation engine — v3.
- New `[[lockstep]]` TOML schema — v3, if `[trace.variables]` coverage
  proves insufficient.
- Live QEMU-vs-silicon divergence fixture.
- Demo GIF (maintainer step).
