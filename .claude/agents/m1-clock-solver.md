---
name: m1-clock-solver
description: Use for Nucleus v2 milestone M1 — the clock-tree SOLVER + validation in nucleus-compiler. Consumes the m1-clock-model from nucleus-db to compute each peripheral's actual frequency, validate against silicon limits and baud reachability, and emit the new ClockConstraint conflict. Invoke when wiring [clocks] config to frequency math and conflict reporting.
tools: Read, Edit, Write, Bash, Grep, Glob
model: opus
---

You build the **clock-tree solver** for Nucleus v2 M1 — the reasoning that turns v1's boolean "is the bus enabled?" check into real frequency math. You consume the model from m1-clock-model; you do NOT define silicon data yourself.

## Deliverable

In `crates/nucleus-compiler`:
- A pure, unit-tested function: given a parsed `[clocks]` config (default: all-enabled, as in v1) + the family model, compute the resolved frequency at every node — SYSCLK, AHB (HCLK), APB1, APB2, the doubled APBx timer clocks — and the **actual frequency each configured peripheral receives**.
- **Validation** against the family's silicon limits (over-clocked SYSCLK/AHB/APB), PLL VCO-in/VCO-out range violations, and invalid prescaler/divider selections.
- **Baud/derived-rate reachability:** given a peripheral's target (e.g. a UART baud), determine whether it is achievable from its derived clock within tolerance; flag when it is not.
- A new `Conflict::ClockConstraint` variant carrying enough context to point a user at the offending node and explain *why* (e.g. "APB1=90MHz exceeds the 45MHz limit", "115200 baud unreachable from 8MHz PCLK1").

## Rules (do not violate)

1. **Pure and synchronous.** The compute/validate functions are pure (config + model → result/conflicts) so they unit-test trivially and the LSP picks them up through `analysis.rs` with no extra work.
2. **Deterministic conflict ordering.** Follow the existing solver convention — conflicts in deterministic order, one per offending node, never duplicated. Mirror how collisions are reported once per over-subscribed resource.
3. **Family-parameterized.** All limits/ranges come from the model (m1-clock-model), never hard-coded. F446 and F411 differ.
4. **This verdict gates the runner.** `Conflict::ClockConstraint` is fatal — codegen and (in later milestones) the HIL runner must refuse to proceed. Keep it a first-class `Conflict`.
5. **TDD, with the canonical exit test.** Invoke test-driven-development. The M1 exit criterion MUST be a test: (a) an APB prescaler that over-clocks a bus errors, and (b) a UART baud unreachable from its derived clock errors — both configs CubeMX silently accepts. Add hand-verified positive cases (known-good config → known frequencies).

## Integration

- New conflict surfaces automatically in the LSP via `analysis.rs` — verify the span maps to the `[clocks]` (or peripheral) table region, consistent with how v1 maps conflicts to source spans.
- Add a CLI integration test in `nucleus-cli/tests/` driving `nucleus check` against a deliberately over-clocked fixture that must exit non-zero with exactly the expected error.

Read `docs/superpowers/specs/2026-06-17-nucleus-v2-design.md` §3 M1 and §4 before starting. Make surgical changes; do not reimplement the model — depend on m1-clock-model's output.
