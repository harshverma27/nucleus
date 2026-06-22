# M4 — Constraint Auto-Router

## Context

M1 (clock-tree), M2 (DMA arbitration), M3 (IRQ/NVIC) all *validate* an explicit, fully-pinned `stm32.toml` and report `Conflict`s when it's wrong. M4 inverts this: the user names the peripheral instances they want (`usart2`, `spi1`, ...) without assigning pins, and a solver searches `nucleus-db`'s pin/AF model — subject to M1–M3's existing constraints — to produce a complete, valid, deterministic pin assignment, written back as a fully-specified `stm32.toml`. Per the v2 design spec and issue #20, this is the "escape hatch" for users who don't want to hand-route pins; M3 just shipped (`4eb1057`) and M4 is the next item on Week 1 (branch `20-v2-week-1-verify-completion-prove-infrastructure`).

This is architecturally new: M1–M3 are all O(1) lookups + linear scans over an already-stated config. M4 is the first true *search* component in the codebase (no backtracking exists anywhere today). `nucleus_db::dma::DmaMap::candidates()` (peripheral+direction → legal slots) and `Conflict::DmaCollision.suggestion` (propose one alternative) are the closest existing precedents, but pins are a strictly harder search: many roles × many candidate pins, with cross-role contention (one pin can only serve one role).

Per CLAUDE.md's GitHub workflow: post this plan as a comment on issue #20 before starting, push directly to `20-v2-week-1-verify-completion-prove-infrastructure`, and post completion (ticking off the M4 checklist) when done.

## Design decisions (resolved with user)

- **Scope: pins only.** The user always names the exact peripheral instance (`usart2`, not "any free USART"); the router only fills in missing pin/AF roles for instances already present in `[peripherals.*]`. No instance-selection search, no new "instance-less" config concept. This matches every example in the design spec and issue #20, which always name instances.
- **Cost function: strict lexicographic priority**, in the order the spec lists: (1) prefer leaving high-demand pins free, (2) minimize DMA pressure, (3) group an instance's signals on one GPIO port. Used as a tie-break ordering, not a weighted sum — fully deterministic, no arbitrary constants.

## Design decisions (resolved during research)

- **Intent syntax: none needed.** Reuse today's schema as-is — an instance present in `[peripherals.<name>]` with a required role's key absent means "route this role"; a key present means "respect this pin as fixed." Zero parser/schema changes (mirrors M2, which added zero new config sections). `[[exti]]` entries stay fully user-specified and are simply treated as pre-occupied pins, like any other already-pinned role — EXTI/interrupt auto-assignment is out of scope for M4.
- **"Minimize DMA pressure" is a documented no-op within pins-only scope.** DMA slot assignment (`DmaMap`) is keyed by peripheral+direction only — it has no dependency on which pin/AF was chosen. This criterion would become meaningful if M4 were later extended to instance-selection (different instances can have different DMA contention), but today it can never break a tie. Document this honestly in the cost-function code comment rather than fabricating a fake signal.
- **New `Conflict::Unroutable { node: String, reason: String }` variant**, reusing the exact `node`-as-peripheral-name shape `IrqConflict`/`ClockConstraint` already use. This gets severity/Display/LSP-diagnostic wiring "for free" by following the established M1–M3 pattern, and means `nucleus route` failures print through the *same* severity-prefixed CLI path as `nucleus check` with zero new printing code.
- **New CLI verb `nucleus route`**, not a flag on `check`. Consistent with v2's established pattern of one verb per milestone (`test`, `history`, `show`, `lockstep` are all separate verbs per CLAUDE.md) and with the spec's "written back as a fully-specified `stm32.toml` the user can inspect and diff" language, which implies a distinct output artifact, not a mutation of `check`'s pass/fail semantics.
- **New dependency: `toml_edit`** (nucleus-compiler only). Today's `toml = "0.8"` (serde-based) round-trips through a `Config` struct and would lose comments/formatting/ordering on re-serialize. `toml_edit::DocumentMut` edits the original text in place (splice in solved pin values, leave everything else byte-identical), which is what "inspect and diff" requires.
- **Backtracking budget: 100,000 trial steps**, then fall back to one-shot greedy (same candidate order, no backtracking/undo). Per issue #20's explicit hint ("CSP solvers can be slow; use a deterministic greedy fallback if backtracking exceeds a depth limit, document the limit"). In practice configs have well under a few dozen open roles with a handful of candidates each, so the budget is a pathological-input safety net, not a normal-path concern.
- **Optimality is implemented as candidate-ordering inside one DFS**, not "enumerate all valid solutions, rank, pick best." Sort each role's candidate pins by the lexicographic cost key before trying them in backtracking order; the first complete assignment found is therefore already cost-optimal by construction. Avoids combinatorial blowup from ranking multiple full solutions.
- **Success requires `route`'s own synthetic fully-assigned config to pass M1–M3 validation internally**, not just by convention. `route_family` runs the existing clock/DMA/IRQ/pin-collision checks on its candidate assignment before reporting success; any conflict here (e.g. DMA stream exhaustion, which is independent of pin choice) becomes the "over-constrained, minimal explanation" failure. This makes issue #20's "Output is itself a valid stm32.toml (passthrough `nucleus check` succeeds)" criterion true by construction, with the fixture test as outside-in confirmation rather than the only guarantee.

## Files

### 1. `crates/nucleus-db/src/lib.rs`
Add `Database::candidate_pins(peripheral: &str, signal: &str) -> Vec<Pin>` — a reverse lookup over the existing `entries: &'static [AfMapping]` slice (`filter` by peripheral+signal, `map` to pin, `collect`), mirroring `DmaMap::candidates()`'s shape exactly: empty vec (not panic) if unmodeled. No new data, no schema change — purely a new accessor over data that already exists. Unit tests alongside, same style as existing `find_af`/`alt_functions` tests.

### 2. `crates/nucleus-compiler/Cargo.toml`
Add `toml_edit` dependency (compiler crate only — CLI and LSP don't need it directly).

### 3. `crates/nucleus-compiler/src/solver.rs`
Add `Conflict::Unroutable { node: String, reason: String }` as a 10th variant. Add its `Display` arm (mirrors `IrqConflict`'s message-formatting style) and its `severity()` arm (always `Severity::Error` — a failed route has no warning-level concept). No changes to `solve()`'s pipeline itself; `Unroutable` is only ever produced by the new router module, never by `solve()`.

### 4. `crates/nucleus-compiler/src/router.rs` (new)
Core CSP module, `pub fn route(config: &Config, db: &Database) -> Result<BTreeMap<(String, String), Pin>, Vec<Conflict>>` (instance, role-key → solved `Pin`; only newly-solved roles, not roles the user already pinned).

- **Open-role extraction**: loop over `config.peripherals` in lexical (`BTreeMap`) order, same as `solve()`'s step 1 — for each role from `model::roles_for(instance)`: key present + parses + `db.find_af` succeeds → mark pin occupied; key present but invalid → hard failure, reusing `InvalidPin`/`AfMismatch` directly (don't try to route around a user's broken explicit pin); key absent → add to the open-roles list. `PeripheralUnavailable`/`ClockDomainDisabled` checks reused the same way — any non-pin problem aborts routing immediately with the existing conflict, before search even starts.
- **Candidate generation + cost ordering**: for each open role, `db.candidate_pins(peripheral, signal)` minus already-occupied pins, sorted by the lexicographic key — (1) ascending pin-demand (count of `AfMapping` entries referencing that pin across the whole DB — fewer references = lower demand = prefer using it now, saving high-demand pins for later), (2) DMA-pressure (constant/no-op per the design decision above, documented inline), (3) same-port-as-a-sibling-role-of-this-instance preference. For multi-role instances, first attempt a whole-port assignment (does one port satisfy every open role of this instance?) before falling back to independent per-role choice.
- **Backtracking DFS**: try the sorted candidates for the first open role, recurse on the rest, undo and try the next candidate on dead-end. Global step counter across the whole search; past 100,000 steps, abort and switch to one-shot greedy (first sorted candidate per role, no undo) as its own directly-callable/testable function.
- **Final validation pass**: on a complete candidate assignment, merge it into a synthetic fully-pinned `Config` and run the existing clock/DMA/IRQ/pin-collision checks (reuse `solve()`'s post-per-peripheral steps directly). Any resulting conflict becomes the failure result.
- **Failure reporting**: on search exhaustion (no candidates left anywhere, greedy fallback also stuck, or the final validation pass above fails), return `Conflict::Unroutable { node, reason }` for the role that got stuck, with a minimal explanation (which candidates existed, what already occupies them) — matching issue #20's example style (`"no free streams for DMA2 TIM2"`).

Unit tests (mirrors `irq.rs`/`dma.rs` style — hand-built small `Config` fixtures): simple successful route; pre-existing hard conflict aborts before search starts; over-constrained roles produce `Unroutable` with a sane message; same-port preference picks a single-port solution over a split-port one when both are valid; demand heuristic picks the lower-demand pin when two valid candidates exist; a case solvable only via actual backtracking (first-choice-looks-good-but-dead-ends); the greedy-fallback function tested directly in isolation (not by actually tripping the 100,000-step counter).

### 5. `crates/nucleus-compiler/src/lib.rs`
Add `pub fn route_family(text: &str) -> Result<(RouteOutcome, String), ParseError>` paralleling `check`/`check_family`: parse, resolve family `Database`, call `router::route`, and on success render the routed TOML via `toml_edit` (parse the *original* text as a `DocumentMut`, splice each solved `(instance, role) -> Pin` in as a string value under the existing `[peripherals.<instance>]` table, serialize back to a string — comments/formatting/ordering elsewhere untouched). On failure, no TOML is rendered — just the conflicts, same shape as `CheckReport`.

### 6. `crates/nucleus-lsp/src/analysis.rs`
Add the `Conflict::Unroutable` arm to `conflict_spans` — mechanically required since `Conflict` is matched exhaustively here. Reuse the exact `IrqConflict` pattern: `name_to_key.get(node).and_then(header_span)`, no text-search fallback needed since `node` is always a peripheral name. (LSP doesn't call `route` today — this is only triggered if `Unroutable` ever reaches `solve()`'s output, which it doesn't — but the match must stay exhaustive.)

### 7. `crates/nucleus-cli/src/main.rs`
Add `Command::Route { path: PathBuf, out: Option<PathBuf> }` (clap) and `run_route()`, mirroring `run_check()`'s read-file → call-into-compiler → print-and-exit shape. On success: write the routed TOML to `out` if given, else print to stdout; `ExitCode::SUCCESS`. On failure: print conflicts via the *existing* severity-prefixed loop (already generic over any `Conflict`, needs zero changes) to stderr; `ExitCode::FAILURE`; no file written.

### 8. Fixtures + CLI integration tests
New fixtures under `tests/fixtures/` (workspace root, same location M3 used): `route_simple.toml` (usart2 + spi1, no pins → deterministic valid assignment), `route_complex.toml` (most/all peripheral kinds → optimal assignment, reproducible), `route_overconstrained.toml` (more demand than the family's pins/AFs can satisfy → clear failure message). New tests in `crates/nucleus-cli/tests/cli.rs`, a `run_route(name)` helper mirroring `run_check(name)`, plus one chained test that routes `route_simple.toml`, writes the output to a tempdir, and feeds that path into `run_check` to assert exit 0 — issue #20's explicit "passthrough" acceptance criterion.

## Verification

- `make check` (fmt-check + lint + test) green
- `cargo test -p nucleus-db` — new `candidate_pins` tests pass
- `cargo test -p nucleus-compiler router` — backtracking/heuristic/fallback unit tests pass
- `cargo run -p nucleus-cli -- route tests/fixtures/route_simple.toml` — manual inspect, deterministic pin choices
- `cargo run -p nucleus-cli -- route tests/fixtures/route_overconstrained.toml` — confirm non-zero exit + minimal, legible failure message
- Chained passthrough: route `route_simple.toml` to a temp file, then `nucleus check` that file — exit 0
- `cargo test -p nucleus-cli` — fixture-based integration tests green

## Rollout

1. Post this plan as a comment on issue #20.
2. Implementation order = the 8 files/commits above (db reverse-lookup → `Unroutable` + `toml_edit` dep → router search core → cost heuristic → `route_family` orchestration + TOML rendering → LSP diagnostic arm → `nucleus route` CLI verb → fixtures/CLI tests).
3. `make check` green → push directly to branch `20-v2-week-1-verify-completion-prove-infrastructure`.
4. Post a completion comment ticking M4's checklist, and tick the same boxes in issue #20's body (not just the comment thread).
