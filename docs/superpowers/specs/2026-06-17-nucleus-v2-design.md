# Nucleus v2 — Design Spec

**Date:** 2026-06-17
**Status:** Approved (brainstorming complete; implementation plan to follow)
**Supersedes:** the 8-phase v1 roadmap in `README.md` (v1 is feature-complete through Phase 8 except maintainer-only launch steps).

---

## 1. Why v2 exists

v1 shipped the full `nucleus` CLI surface (`check`/`init`/`build`/`flash`/`lsp`/`trace`), a deterministic pin/AF/peripheral database (NUCLEO-F446RE + NUCLEO-F411RE), a constraint solver, HAL codegen, an ITM/SWO decoder, a WebSocket trace pipeline, a React trace dashboard, and Phase 7 distribution/CI automation.

The public critique that v2 must answer is blunt and specific:

> "Why does this exist when CubeMX exists? It's just CubeMX with a command line."

On the surface, v1 invites that read: it muxes pins, resolves alternate functions, and generates `HAL_*_Init` calls — all things CubeMX does through a GUI. The rebuttal cannot be "same thing, in a terminal." It has to be a **category CubeMX structurally cannot enter.**

**The v2 thesis (one sentence):**

> **CubeMX writes code and walks away. Nucleus *proves your hardware works* — statically, then on real silicon and a simulated twin, in your pull request.**

CubeMX is a one-shot GUI wizard that emits plausible-but-unverified code and disappears. Nucleus v2 is a **correct-by-construction, lifecycle-long reasoning engine** that lives in CI, in code review, and against running hardware. Every milestone below earns its place by answering "why does this exist."

### Explicitly deferred (NOT in v2)

- **UI / TUI / "feel" work.** No living-board TUI, no fancy terminal rendering, no new dashboard surface beyond reusing existing Canvas charts for history graphs. Deferred to a later version by decision.
- **Additional MCU families.** v2 stays on the two existing boards (F446RE, F411RE). Broadening the family matrix is a later-version concern. v2 proves *depth*, not *breadth*.
- **Cloud registry / upload features.** Still a local-only tool (v1 Scope Discipline Rule 5 holds).

---

## 2. The correctness loop

v2 is organized as a four-stage loop, plus a crowning capability that closes it:

```
        ┌──────────────────────────────────────────────────────────┐
        │                                                          │
   VERIFY ──────▶ SOLVE ──────▶ PROVE ──────▶ REMEMBER             │
   (flag wrong   (build it     (run on       (record which        │
    config        for you)      QEMU + real   version passed       │
    before run)                 silicon)      which tests)         │
        │                                                          │
        └────────────────────▶ LOCKSTEP ◀──────────────────────────┘
                    (sim and metal agree — or here's
                     the exact instant they diverge)
```

**Cross-cutting invariant (binding):** *The runner never executes a configuration the Verifier rejects.* Wrong code is flagged **before** it is ever flashed or emulated. This is the integration seam between VERIFY and PROVE, and it is the single most important behavioral guarantee in v2: a skeptic watches nucleus reject broken code *and* prove good code in one flow.

---

## 3. Pillars and the ten milestones

Each milestone has a **measurable exit criterion** (matching v1's gated-phase culture). Milestones are sequenced so each builds on the last; the crown (M10) is the apex that requires the substrate beneath it.

### Pillar I — VERIFY (catch bugs CubeMX ships)

The Verifier turns nucleus from "is the bus clock enabled?" (v1's only clock check) into a real hardware reasoning engine that rejects configurations that *compile and flash but do not work*.

**M1 — Clock-tree solver.**
Model the full STM32F4 clock tree: oscillator sources (HSE/HSI/LSE/LSI), the main PLL (M/N/P/Q dividers) and PLLI2S/PLLSAS as applicable, SYSCLK selection, the AHB prescaler, APB1/APB2 prescalers, and per-peripheral clock derivation (including the APBx timer ×2 rule). Given a `[clocks]` config, compute the *actual* frequency every configured peripheral receives and validate it against intent and silicon limits (e.g. APB1 ≤ 45 MHz, APB2 ≤ 90 MHz, SYSCLK ≤ 180 MHz on F446; the F411 limits differ and must be family-parameterized).
- **Replaces** v1's boolean "bus enabled" check with frequency math.
- **Exit:** nucleus catches (a) an APB prescaler that over-clocks a bus, and (b) a UART baud rate that is unreachable from its derived clock — both of which CubeMX silently accepts. Unit-tested against hand-verified reference frequencies for both families.

**M2 — DMA arbitration.**
Model the DMA controllers (DMA1/DMA2), their streams × channels, and the peripheral-request mapping table (which peripheral/direction maps to which stream+channel). Detect when two enabled peripherals require the same stream, and — using the request map — suggest a conflict-free alternative when one exists.
- **Exit:** a config with two peripherals contending for one DMA stream produces exactly one error that names both peripherals and proposes a valid alternative assignment. Deterministic ordering, unit-tested.

**M3 — IRQ / NVIC verifier.**
Model the NVIC vector table, shared interrupt lines (notably the EXTI line groupings, e.g. EXTI9_5/EXTI15_10), priority grouping, and the codegen's handler ownership. Catch: two GPIO interrupts sharing an EXTI group without disambiguation, an interrupt enabled with no generated handler, and priority inversions between dependent peripherals (e.g. a DMA-completion ISR lower priority than the peripheral it serves).
- **Exit:** an EXTI shared-line collision and an enabled-but-unhandled IRQ are both caught and reported with the offending peripherals named.

> **Verifier integration:** M1–M3 extend the existing `Conflict` enum in `nucleus-compiler`. They feed the LSP (new diagnostics surface automatically through `analysis.rs`) and, critically, the HIL runner's pre-flight gate (the cross-cutting invariant). New conflict variants: `ClockConstraint`, `DmaCollision`, `IrqConflict`.

### Pillar II — SOLVE (do the wiring CubeMX won't)

**M4 — Constraint auto-router.**
Invert the verifier. The user declares *intent* — "I need USART2, SPI1, one ADC channel, and two PWM outputs" — without naming pins. A backtracking constraint-satisfaction solver searches the unified model (pin/AF from `nucleus-db`, plus the clock/DMA/IRQ constraints from M1–M3) and produces a complete, valid, and *optimal* assignment (optimality = a documented, deterministic cost function: prefer leaving high-demand pins free, minimize DMA pressure, keep related signals on one port). The result is written back as a fully-specified `stm32.toml` the user can inspect and diff.
- **Exit:** a config that specifies peripherals but zero pins resolves to a complete valid assignment; an over-constrained request fails with a minimal explanation of what could not be satisfied. Deterministic output, unit-tested with golden fixtures.

### Pillar III — PROVE (dual-backend HIL — the killer)

This pillar is the category CubeMX cannot enter at all: actually running the firmware and asserting the silicon did the thing.

**M5 — Dual-backend HIL substrate.**
One runner abstraction, two interchangeable backends behind a single trait/interface:
- **Emulator backend (QEMU):** boot the built firmware in a QEMU machine model for the target, drive/observe via the same ITM stream and an emulated pin/peripheral surface. Runs anywhere, no hardware, every PR.
- **Hardware backend (SWD/ITM):** flash the real board (`st-flash`/OpenOCD), observe via the existing `nucleus-itm` decoder and SWD register/pin reads.

Both backends expose the same observation API (read pin state, read register, await ITM event, sample over a window). The default is "run both"; the user can select one. The runner consumes the build artifacts produced by the existing `nucleus build`/codegen path.
- **New crate:** `nucleus-hil` (host-side runner + backend trait + both backend impls). Reuses `nucleus-itm`, `nucleus-trace::source`, and the OpenOCD plumbing.
- **Exit:** nucleus can, from one command, flash-or-emulate a firmware, observe a pin toggle and an ITM event, and return a structured `RunResult` from *each* backend.

**M6 — Declarative tests (model A).**
`[[test]]` blocks authored alongside `stm32.toml`, version-controlled and diffable. Assertion vocabulary (declarative, no C):
- **Pin-level:** `pin PA5 toggles at 1Hz ±5%`, `pin PA5 is high within 10ms`, edge counts over a window.
- **Protocol-level:** `UART2 echoes "ping" within 10ms`, `I2C1 device 0x68 ACKs`.
- **Timing windows** and **ITM-event** assertions (`trace event "boot_done" within 50ms`).
The compiler parses and validates `[[test]]` (a known peripheral, a reachable pin, sane tolerances); the runner executes each assertion on the selected backend(s) and reports pass/fail.
- **`nucleus test`** is the new CLI verb; exits non-zero on any failure (CI-gatable, like `check`).
- **Exit:** a fixture repo with declarative tests runs `nucleus test` and reports per-assertion pass/fail on **both** QEMU and hardware, exit-code gated.

**M7 — Scripted tests (model B).**
The power-user escape hatch: a small **device-side test agent** (a minimal, optional firmware component exposing a command protocol over a debug channel — set GPIO, read GPIO, read register, trigger a peripheral op) plus a **host SDK** (Rust first) that drives it. This enables stimulus/response tests beyond static assertions: drive an input, read an output, assert a relationship; loopback tests; multi-step sequences.
- The agent is opt-in and isolated; it never ships in production firmware. Protocol is documented and versioned.
- **Exit:** a Rust-authored test drives UART2 TX and asserts the echoed RX over a loopback, passing on **both** backends.

### Pillar IV — REMEMBER (nucleus owns the project)

**M8 — Project ledger.**
Nucleus treats *every* project as a tracked repository in its own right — independent of (but complementary to) git. A local `.nucleus/` store content-addresses each **version** = hash of (`stm32.toml` + resolved config + firmware binary + toolchain identity) and records, per version: the verifier verdict, the solved assignment, and **which tests passed on which backend, when**. The ledger is append-only and deterministic.
- **New verb:** `nucleus history` (list versions and their results), `nucleus show <version>`.
- **Storage:** a small, documented on-disk format under `.nucleus/` (JSON or SQLite — decided in the implementation plan; must be diff-friendly or fully tool-managed, and `.gitignore`-able by default with an opt-in to commit).
- **Exit:** across several commits/edits, `nucleus history` shows each version with its per-backend test results; re-running an unchanged version is recognized as the same version (content-addressing works).

**M9 — History graphs + CI-native HIL.**
Two halves that ship together:
- **Visualization:** trend graphs of pass/fail counts, assertion timing, and CPU-load across versions over time. **Reuses the existing dashboard's Canvas chart components** (Phase 6) with the ledger as a new data source — *no new UI category*, consistent with deferring "feel" work. A `nucleus history --graph` opens the existing dashboard in a "history" mode.
- **CI-native HIL:** extend the Phase 7 composite action (`.github/actions/nucleus`) to run the full loop in CI — verify, build, run both backends (emulator always; hardware when a self-hosted runner with a board is available) — and post a ledger-backed report to the PR: conflict count, firmware size, **per-backend test results**, the trend graph, and a **signed hardware verification report** (a machine-checkable artifact stating exactly what was verified statically and what was proven on which silicon).
- **Exit:** a PR shows "12 passed on QEMU, 12 passed on NUCLEO-F446RE" plus a trend graph and an attached verification report.

### The Crown

**M10 — Lockstep Co-Execution (the Divergence Detector) + launch.**
The capability that has never existed in STM32 history: run the *same* firmware on the QEMU twin **and** the real silicon, compare them at shared sync points, and automatically detect, locate, and explain the **first divergence**.

- **Honest scope — observation-level, not cycle-exact.** QEMU is not cycle-accurate, so lockstep is defined over *observable sync points* nucleus already controls: ITM events, register snapshots taken at matching breakpoints, and pin/assertion states sampled at declared instants. We compare state at these checkpoints, not every cycle. This is what makes it buildable *and* it is still unprecedented as a turnkey STM32 tool.
- **Output — the divergence report:** the first checkpoint at which the twin and the metal disagree, the diverging observable (e.g. `TIM2->CNT` sim=0x0410 vs silicon=0x0412), and a best-effort *explanation* drawn from the model (clock-source skew, a known erratum, a peripheral the emulator models imperfectly, a race). When they agree to the end, that agreement is itself the certificate.
- **Why it ends the Reddit thread:** CubeMX has no runtime, no twin, and no silicon link — it cannot conceive of this. The launch GIF is split-screen **sim | silicon**, same firmware, a scrubbed timeline, then a red marker at the first disagreement with its named cause.
- **Launch:** the public demo, README/`docs/` rewrite around the correctness loop, and the recorded GIF that answers the critique directly. (Recording is a maintainer step; the capability and artifacts are the milestone deliverable.)
- **Exit:** for a deliberately divergence-inducing firmware (e.g. one that depends on a timing detail QEMU models differently), nucleus reports the correct first divergence point and observable; for a clean firmware, it certifies agreement end-to-end. Both paths integration-tested (the hardware leg is a maintainer step on a physical board; CI validates the emulator leg and the comparison logic against captured/synthetic silicon traces).

---

## 4. Architecture

v2 is additive to the existing multi-crate workspace. It does **not** violate v1's binding Scope Discipline Rules: the extension stays a thin display layer (Rule 1), codegen still calls only stock `HAL_*_Init` (Rule 2), and there is no cloud (Rule 5). The "no DMA collision / no clock-tree solver" limits (Rule 4) were *v1-phase* scope; v2's entire VERIFY pillar is the deliberate lifting of that limit and is the headline value, not scope creep.

### Crate map

| Crate | v1 role | v2 changes |
|---|---|---|
| `nucleus-db` | pin/AF/peripheral tables | add clock-tree, DMA-request-map, and NVIC/EXTI model data (build-time generated where pack data allows; hand-maintained tables where it doesn't, with the same patch-table discipline as `pack.rs`). |
| `nucleus-compiler` | parser + solver + codegen | new `Conflict` variants (`ClockConstraint`, `DmaCollision`, `IrqConflict`); the M4 auto-router; `[[test]]` parsing/validation. The peripheral model (`model.rs`) gains DMA/IRQ/clock attributes. |
| `nucleus-cli` | command dispatch | new verbs `test`, `history`, `show`, `lockstep` (names finalized in the plan); orchestrates the HIL runner and ledger. |
| `nucleus-lsp` | diagnostics/hover/completion | new conflicts surface automatically via `analysis.rs`; hover/completion extended for `[[test]]` blocks. |
| `nucleus-itm` | ITM/SWO decoder | unchanged interface; reused by both HIL backends. |
| `nucleus-trace` | trace pipeline + WS server | `source.rs` reused by the HIL hardware backend; dashboard charts reused for history graphs. |
| **`nucleus-hil`** *(new)* | — | host-side runner; backend trait + QEMU and hardware backends; observation API; the M10 lockstep comparator. |
| **`nucleus-ledger`** *(new, or a module of cli)* | — | content-addressed version store, `.nucleus/` format, query API for `history`. |
| device test-agent *(new, optional target component)* | — | minimal on-target command protocol for scripted tests (M7); never in production builds. |
| `extension/` | LSP client + dashboard | unchanged thin-client role; dashboard gains a history-mode data source only. |

### Data flow (the loop, end to end)

```
stm32.toml ─▶ nucleus-compiler ─▶ Verifier verdict ──(reject)──▶ stop, report conflicts
                   │                     │
                   │                     └──(accept)──┐
                   ▼                                  ▼
            auto-router (M4)                    nucleus build (codegen + CMake/gcc)
                   │                                  │
                   ▼                                  ▼
            resolved stm32.toml              firmware.bin ──▶ nucleus-hil runner
                                                              │            │
                                                       QEMU backend   HW backend
                                                              │            │
                                                        RunResult     RunResult
                                                              └────┬───────┘
                                                                   ▼
                                                          [[test]] assertions / scripts
                                                                   ▼
                                                            nucleus-ledger (version ↦ results)
                                                                   ▼
                                                   history graphs + CI report + lockstep diff
```

### Error handling & safety

- **Verifier rejection is fatal to the run** (the invariant). `nucleus test`/`build` exit non-zero; nothing is flashed or emulated.
- **Missing toolchain / missing QEMU / no board** each yields a clear, specific error and degrades gracefully: the emulator leg runs without hardware; the hardware leg is skipped (not failed) with an explicit "no board detected" status, exactly as v1 handles a missing cross toolchain (codegen still observable).
- **HIL never bricks a board:** flashing uses the existing `st-flash`/OpenOCD path; the test agent is opt-in and isolated.
- **Determinism:** verifier output, auto-router output, ledger versioning, and codegen remain byte-deterministic for testable CI.

### Testing strategy (per pillar)

- **VERIFY (M1–M3):** unit tests against hand-verified reference values (clock frequencies, DMA request maps, EXTI groupings) for both families — the same "hand-verified seed" discipline as `nucleus-db`'s `SEED`.
- **SOLVE (M4):** golden-fixture tests; over-constrained cases assert minimal failure explanations.
- **PROVE (M5–M7):** the emulator backend is fully CI-testable; declarative and scripted suites run end-to-end in QEMU in CI; the hardware leg is a maintainer step (physical board) validated against captured/replayed silicon traces in CI.
- **REMEMBER (M8–M9):** ledger round-trip and content-addressing unit tests; CI-report rendering tested against fixtures.
- **CROWN (M10):** the comparator and divergence-localization logic are unit/integration tested against synthetic paired traces (one clean, one deliberately divergent); the live silicon leg is a maintainer step.

---

## 5. What stays true from v1 (non-negotiables carried forward)

- Extension contains **zero business logic** (Rule 1). History graphs reuse existing chart components fed by CLI-produced data; the lockstep diff is computed in Rust.
- Codegen calls **only stock `HAL_*_Init`** (Rule 2). HIL adds *observation*, not HAL reimplementation. The optional test agent is separate from generated init code.
- **Local-only** (Rule 5): the ledger, graphs, and reports are all on-disk/CI artifacts; no upload, no registry.
- **Deterministic, gated milestones** with measurable exit criteria, like v1's phases.

---

## 6. Out of scope for v2 (named to prevent scope creep)

- Living-board TUI / any new "feel" surface (deferred by decision).
- MCU families beyond F446RE and F411RE.
- Cycle-accurate emulation (lockstep is observation-level by design).
- Full clock-*tree configuration UX* beyond what M1 needs to verify; M1 *verifies* the tree, it does not become a graphical clock configurator.
- Cloud anything.

---

## 7. Open questions for the implementation plan

These are deliberately deferred to `writing-plans`, not left vague in scope:

1. Ledger storage format: JSON-on-disk vs. embedded SQLite (trade diff-friendliness vs. query power).
2. QEMU machine model fidelity per board — which peripherals are observable in-emulator vs. hardware-only, and how M10 accounts for emulator-imperfect peripherals in its explanations.
3. Device test-agent transport (RTT vs. a dedicated ITM port vs. semihosting) and its protocol versioning.
4. Final CLI verb names (`test`/`history`/`show`/`lockstep`) and their flag surfaces.
5. Exact cost function for the M4 auto-router's "optimal" assignment.
