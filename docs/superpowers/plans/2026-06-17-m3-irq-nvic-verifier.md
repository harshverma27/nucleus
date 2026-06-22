# M3 — IRQ / NVIC Verifier

## Context

v2 Week 1 (issue #20) needs M3 before M4/M5 can build on it (M4's CSP solver consumes M1–M3 constraints; M5's runner gates on `nucleus check`). M1 (clock-tree) and M2 (DMA arbitration) are done and set the pattern: a hand-maintained data model in `nucleus-db` (cross-validated by a reference-manual seed test), a pure solver module in `nucleus-compiler` that turns config + model into `Conflict`s, wired into `solver::solve()`, surfaced through `nucleus-lsp::analysis.rs`, and end-to-end CLI fixture tests.

M3 catches two concrete silent-fail classes CubeMX lets through: an interrupt enabled in config that Nucleus has no NVIC vector for (so the CPU just spins in the default weak handler forever), and two GPIO pins on different ports sharing one EXTI line (the SYSCFG_EXTICR mux only routes one port per line number — wiring both is a real hardware conflict, not a style nit). A third, lower-stakes check (DMA ISR configured less urgent than the peripheral it serves) is a warning, not an error — this is the first conflict severity that must not fail `nucleus check`, so `Conflict` gains a severity concept.

Per CLAUDE.md's GitHub workflow: post this plan as a comment on issue #20 before starting, push directly to `20-v2-week-1-verify-completion-prove-infrastructure`, and post completion (ticking off the M3 checklist) when done.

## Design decisions (resolved with user)

- **EXTI surface:** new `[[exti]]` array-of-tables in `stm32.toml` (`pin = "PA0"`, optional `priority = N`), not a new GPIO peripheral kind and not deferred.
- **"Unhandled IRQ" meaning:** verifier-only. `irq = true` opt-in on a peripheral Nucleus's NVIC model doesn't cover for the family is the error. No codegen change (no ISR-stub generation) — stays in scope for a verifier milestone, doesn't touch Rule 2 territory.
- **Priority inversion:** optional `irq_priority` / `dma_priority` keys (0–15, smaller = more urgent, matching ARM NVIC). Only checked when both are explicit on the same peripheral table; absent means skip (never a false positive), matching the `peripheral_bus`/DMA `has_peripheral` discipline of "never guess."

## Files

### 1. `crates/nucleus-db/src/irq.rs` (new)

Mirrors `dma.rs`'s shape:

- `ExtiGroups`: line→NVIC-vector-name lookup. Lines 0–4 are individual vectors (`EXTI0`..`EXTI4`); lines 5–9 share `EXTI9_5`; lines 10–15 share `EXTI15_10`. Identical on both families (same NVIC layout in RM0390/RM0383). A `const fn group_for(line: u8) -> &'static str`.
- `PeripheralIrq { peripheral: &'static str, vectors: &'static [&'static str] }` — one row per peripheral Nucleus already models in `nucleus_compiler::model` (USART1/2/3, UART4/5, USART6, SPI1–4, I2C1–3, TIM2–5 — the kinds `roles_for` covers). Most have one vector; I2Cx has two (`I2Cx_EV`, `I2Cx_ER`).
- `IrqMap` struct with `f446re()` / `f411re()` constructors (mirrors `DmaMap`), `has_peripheral(&self, peripheral: &str) -> bool`, `vectors(&self, peripheral: &str) -> &'static [&'static str]`.
- Doc comment citing RM0390/RM0383 vector table sections, same "hand-maintained, never guess" framing as `dma.rs`'s header.
- Seed tests: known peripherals present on F446, absent on F411 (UART4/5 mirrors the existing `family_parameterized_f411` DMA test pattern), EXTI group boundaries correct (line 4 → `EXTI4`, line 5 → `EXTI9_5`, line 9 → `EXTI9_5`, line 10 → `EXTI15_10`).

Register in `crates/nucleus-db/src/lib.rs`: `pub mod irq;`.

### 2. `crates/nucleus-compiler/src/config.rs`

- Add `pub exti: Vec<ExtiPin>` field to `Config` (`#[serde(default)]`, TOML array-of-tables `[[exti]]` maps natively).
- New struct: `ExtiPin { pub pin: String, #[serde(default)] pub priority: Option<u8> }` with `#[serde(deny_unknown_fields)]`, same style as `TraceVariable`.
- No change to `Peripheral` — it's already a transparent raw bag, so `irq`, `irq_priority`, `dma_priority` keys need no schema change; the solver reads them via `table.0.get(...)` exactly like `dma.rs` reads `dma`.

### 3. `crates/nucleus-compiler/src/irq.rs` (new)

Mirrors `dma.rs`'s `validate(config, map) -> Vec<Conflict>` shape, called from `solver::solve()` after the DMA step.

- **Unhandled IRQ:** for each peripheral table with `irq = true` (or `irq = false`/absent → skip, same boolean-opt-in convention as `dma`), resolve the DB peripheral name; if `!map.has_peripheral(name)`, push `Conflict::IrqConflict` (error) naming it. If modeled, clean.
- **EXTI collision:** parse `config.exti` pins (skip unparsable — `InvalidPin`-style values are not this module's job; reuse `Pin::from_str` and ignore failures silently here since pin validity isn't EXTI's concern... actually: surface bad EXTI pin strings too, mirroring `InvalidPin`, via a small local check). Group by `pin.number` (the EXTI line, 0–15, shared across all 8 ports). Any line claimed by ≥2 distinct ports → one `Conflict::IrqConflict` (error) per contested line, naming both pins. One conflict per line (not per pair), matching the existing `PinCollision`/`DmaCollision` dedup discipline.
- **Priority inversion:** for each peripheral with both `dma = …` and `dma_priority` set, and `irq_priority` also set on the same table, compare: if `dma_priority > irq_priority` (DMA serves it but is numerically less urgent), push `Conflict::IrqConflict` (warning) naming the peripheral and both priority values. Skip if either key is absent.

Unit tests (mirror `dma.rs`'s test module): clean config no conflicts; modeled peripheral `irq=true` clean; unmodeled peripheral `irq=true` → error; two EXTI pins same line different ports → exactly one collision naming both; different lines → clean; priority inversion fires only when both keys present and inverted; F411-vs-F446 family parameterization (an F446-only peripheral's `irq=true` errors on F411 the same way `PeripheralUnavailable` does — or simply unmodeled there too).

### 4. `crates/nucleus-compiler/src/solver.rs`

- New `Severity` enum: `Error`, `Warning`. `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`.
- New `Conflict::IrqConflict { node: String, reason: String, severity: Severity }` — reuses the `ClockConstraint { node, reason }` shape (already proven for "several distinct conditions, one flexible variant") plus the new severity field. `node` is either a DB peripheral name (unhandled/priority-inversion cases, for `name_to_key` lookup in the LSP) or a pin string like `"PA0"` (EXTI collision case, for a text-search fallback).
- `Conflict::severity(&self) -> Severity`: `IrqConflict { severity, .. } => *severity`, every other existing variant `=> Severity::Error` (preserves current behavior exactly).
- `Display` arm: `"IRQ conflict [{node}]: {reason}"`.
- Wire into `solve()`: `conflicts.extend(crate::irq::validate(config, &irq_map));` after the DMA line, using a new `irq_map_for(family)` helper in `lib.rs` (mirrors `dma_map_for`/`clock_tree_for`).
- Existing tests untouched; add one asserting `Conflict::PinCollision { .. }.severity() == Severity::Error` etc. to lock in the "old variants are still all errors" invariant.

### 5. `crates/nucleus-compiler/src/lib.rs`

- `pub fn irq_map_for(family: &str) -> nucleus_db::irq::IrqMap` mirroring `dma_map_for`.
- `pub use solver::Severity;` (re-export alongside `Conflict`).
- `CheckReport::is_ok()`: change from `conflicts.is_empty()` to `!conflicts.iter().any(|c| c.severity() == Severity::Error)` — warnings no longer fail a check.

### 6. `crates/nucleus-cli/src/main.rs` (`run_check`)

- Keep `report.is_ok()` as the exit-code gate (now severity-aware via the `lib.rs` change above — no logic duplicated here).
- When printing the conflict list, prefix each with `"  error: "` or `"  warning: "` based on `conflict.severity()`, instead of the current hardcoded `"  error: "`.
- When `report.is_ok()` but `report.conflicts` is non-empty (warnings only), still print the "OK" success line but follow it with the warning list, and still return `ExitCode::SUCCESS`.

### 7. `crates/nucleus-lsp/src/analysis.rs`

- Generalize the `error()` helper to take a `DiagnosticSeverity` (or add a sibling `warning()`); `diagnostics()` picks ERROR/WARNING from `conflict.severity()`.
- New `conflict_spans` arm for `Conflict::IrqConflict { node, .. }`: try `name_to_key.get(node)` → `header_span` (covers the peripheral-keyed cases) `.or_else(|| find pin text anywhere in the doc)` for the EXTI-pin case — add a small `find_pin_anywhere(text, pin_str)` helper (same quoting logic as `find_quoted`, but searched over the whole document instead of one table's region) → fallback `whole_first_line`.
- Tests mirroring the existing `dma_collision_underlines_first_peripheral_table` / `clock_constraint_underlines_clocks_table` pair: one for unhandled-IRQ (underlines peripheral header, ERROR), one for EXTI collision (underlines the first colliding pin's `[[exti]]` line, ERROR), one for priority inversion (WARNING severity on the diagnostic).

### 8. Fixtures + CLI integration test

- `tests/fixtures/exti_collision.toml`: `[[exti]]` entries for `PA0` and `PB0` (same line, different ports) — the exact fixture path the issue's acceptance criterion names.
- `tests/fixtures/irq_unhandled.toml`: a peripheral kind with no modeled NVIC vector (or a peripheral disabled on the target family) with `irq = true`.
- `crates/nucleus-cli/tests/cli.rs`: two new tests mirroring `dma_collision_exits_nonzero_with_suggestion` — both fixtures exit 1 via `nucleus check`.

## Verification

- `make check` (fmt-check + lint + test) green.
- `cargo test -p nucleus-db irq`, `cargo test -p nucleus-compiler irq`, `cargo test -p nucleus-lsp` for the new modules specifically.
- `cargo run -p nucleus-cli -- check tests/fixtures/exti_collision.toml` → exit 1, message names both `PA0`/`PB0`.
- `cargo run -p nucleus-cli -- check tests/fixtures/dma_collision.toml` (existing M2 fixture) still exits 1 unchanged — confirms the severity refactor didn't regress M1/M2.
- `cargo run -p nucleus-cli -- check tests/fixtures/clean.toml` still exits 0.

## Rollout

1. Post this plan as a comment on issue #20.
2. Implement in the order above (db model → config → compiler solver → solver.rs severity plumbing → cli → lsp → fixtures/tests).
3. `make check` green, then push directly to `20-v2-week-1-verify-completion-prove-infrastructure` (per CLAUDE.md's Week 1 workflow — no PR, direct push).
4. Post completion comment on #20 ticking the M3 checklist items.
