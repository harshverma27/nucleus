//! The constraint auto-router (Nucleus v2 milestone M4).
//!
//! M1–M3 all *validate* an explicit, fully-pinned `stm32.toml` and report
//! [`Conflict`]s when it's wrong. M4 inverts the problem: the user names a
//! peripheral instance (`usart2`, `spi1`, ...) without assigning some or all
//! of its pins, and [`route`] searches [`Database`]'s pin/AF model for a
//! complete, valid, deterministic assignment for exactly the roles left open.
//!
//! This is the first true *search* component in the codebase — M1–M3 are all
//! O(1) lookups + linear scans over an already-stated config. The closest
//! existing precedent is [`crate::dma::validate`]'s greedy first-fit over
//! [`nucleus_db::dma::DmaMap::candidates`]; this module is a strictly harder
//! version of the same idea (full undo/recursion instead of first-fit-only),
//! because one pin can only serve one role, so an early greedy choice can
//! starve a later role that has no other option.
//!
//! ## Scope: pins only
//!
//! The user always names the exact peripheral instance; the router never
//! chooses *which* instance to use, only *which pins*. A required role with
//! its key absent means "route this role"; a key present means "respect this
//! pin as fixed" (validated exactly like [`crate::solver::solve`] would, and
//! any pre-existing hard conflict aborts routing before search even starts).
//! An *optional* role with its key absent is left alone — matching
//! [`crate::solver::solve`]'s own discipline that a missing optional pin is
//! never an error, so the router never force-allocates a pin for a signal the
//! user has no use for. `[[exti]]` entries are pre-occupied pins, treated
//! like any other already-pinned role — there is no EXTI auto-routing in M4.
//!
//! ## Cost function
//!
//! Candidate pins for each open role are sorted by a **strict lexicographic
//! priority** (not a weighted sum, so there are no arbitrary tuning
//! constants):
//!
//! 1. **Ascending pin-demand** — the number of [`nucleus_db::AfMapping`] rows
//!    referencing that pin across the whole database (via
//!    [`Database::alt_functions`]). Fewer references means lower demand:
//!    using a low-demand pin now keeps high-demand (more contested) pins free
//!    for later roles.
//! 2. **DMA-pressure — a documented no-op.** [`nucleus_db::dma::DmaMap`] keys
//!    its candidate slots by `(peripheral, direction)` only; it has no
//!    dependency on which pin or AF number was chosen for that peripheral's
//!    signals. So this criterion can never break a tie in the pins-only scope
//!    M4 ships — it is listed because the design spec's cost function names
//!    it, and kept as an explicit, always-equal comparison rather than
//!    silently dropped, so a future instance-selection extension (where DMA
//!    contention *would* vary) has an obvious place to plug in.
//! 3. **Same-port-as-a-sibling-role preference.** For a multi-role instance,
//!    [`preferred_port`] checks whether one GPIO port has a free candidate for
//!    *every* still-open role of that instance; if so, every role's sort
//!    prefers that port. Because this is the lowest-priority key, it only
//!    breaks ties left by demand — it can never override a clear demand
//!    winner. Trying the shared-port candidate first for every role of the
//!    instance is what makes the backtracking search attempt a whole-port
//!    assignment before falling back to independent per-role choices: it
//!    falls out of the sort order, rather than needing a separate "try one
//!    port" phase.
//!
//! ## Search
//!
//! [`route`] runs a backtracking depth-first search over the open roles (in
//! the same lexical/per-role order [`crate::solver::solve`] visits them):
//! try the sorted candidates for the first open role, recurse on the rest,
//! undo and try the next candidate on dead-end. A global step counter spans
//! the whole search; past [`BACKTRACK_BUDGET`] steps, the search aborts and
//! [`route`] falls back to [`route_greedy`] — the same sorted candidate
//! order, first candidate per role, no undo. [`route_greedy`] is also a
//! directly callable function in its own right (used by tests that want to
//! exercise the no-backtracking strategy without driving the search to the
//! budget).
//!
//! On a complete candidate assignment, [`route`] merges it into a synthetic
//! fully-pinned [`Config`] and re-runs the clock/DMA/IRQ/pin-collision checks
//! [`crate::solver::solve`] itself runs, so a successful route is valid by
//! construction, not just by the router's own bookkeping. Any failure from
//! search exhaustion, greedy exhaustion, or this final validation pass is
//! reported as [`Conflict::Unroutable`].

use std::collections::BTreeMap;
use std::str::FromStr;

use nucleus_db::{Database, Pin, Port};

use crate::config::Config;
use crate::model::{self, Role};
use crate::solver::{self, Conflict};

/// Pins already occupied, mapped to a short owner label for diagnostics
/// (`"usart2.tx"` for an already-pinned role, `"[[exti]]"` for an EXTI entry).
/// Grown during the search as the DFS/greedy strategy tentatively commits
/// candidates.
type Occupied = BTreeMap<Pin, String>;

/// The backtracking step budget. Past this many candidate trials across the
/// whole search, [`route`] gives up on optimality and falls back to
/// [`route_greedy`]. Configs in practice have well under a few dozen open
/// roles with a handful of candidates each, so this is a pathological-input
/// safety net, never a normal-path concern (per issue #20's explicit hint to
/// document the limit).
const BACKTRACK_BUDGET: u64 = 100_000;

/// One unresolved pin role: a peripheral instance's role left without a pin
/// in the config, the database peripheral/signal it must resolve, and its
/// sorted candidate pins (filtered to exclude pins already occupied when the
/// roles were extracted — narrowed further, live, during the search as
/// other roles commit pins).
#[derive(Debug, Clone)]
struct OpenRole {
    /// The config key, e.g. `"usart2"`.
    instance: String,
    /// The database peripheral name, e.g. `"USART2"`.
    peripheral: String,
    /// The `stm32.toml` key, e.g. `"tx"`.
    key: &'static str,
    /// The database signal name, e.g. `"TX"`.
    signal: &'static str,
    /// Candidates, sorted by the cost key, with already-occupied pins
    /// removed. Static for the duration of one search: the *order* never
    /// changes, but the search must still skip a candidate that another role
    /// claimed since this list was built.
    candidates: Vec<Pin>,
}

impl OpenRole {
    fn node(&self) -> String {
        format!("{}_{}", self.peripheral, self.signal)
    }
}

/// Where the search got stuck, for [`Conflict::Unroutable`]'s message: which
/// role ran out of candidates, and what every one of its candidates was
/// already assigned to.
struct StuckAt {
    role_node: String,
    /// `(candidate pin, what already occupies it)` — `None` for a candidate
    /// that simply never existed (the role had zero candidates from the
    /// database to begin with).
    candidates_considered: Vec<(Pin, Option<String>)>,
}

impl StuckAt {
    fn reason(&self) -> String {
        if self.candidates_considered.is_empty() {
            return "no candidate pins are modeled for this signal on this family".to_string();
        }
        let parts: Vec<String> = self
            .candidates_considered
            .iter()
            .map(|(pin, holder)| match holder {
                Some(h) => format!("{pin} (held by {h})"),
                // Should not occur when `stuck_at` is built from a genuine
                // dead-end (every candidate was occupied) -- kept as a
                // defensive, honest label rather than panicking if it ever
                // does (e.g. a future caller invoking it speculatively).
                None => format!("{pin} (apparently free)"),
            })
            .collect();
        format!("no free pin among candidates: {}", parts.join(", "))
    }
}

/// Search for a complete, valid pin assignment for every required role left
/// open in `config`, against `db`. Returns only the *newly* solved
/// `(instance, role key) -> pin` pairs — not the roles the user already
/// pinned.
///
/// `Err` covers two distinct situations, both signaled before or after search
/// rather than mid-search:
/// - A pre-existing hard conflict in an already-pinned role (invalid pin
///   syntax, an AF mismatch, an unavailable peripheral, or a disabled clock
///   domain) aborts immediately with that conflict, reused directly — the
///   router never tries to route *around* a user's broken explicit pin.
/// - Search exhaustion (no candidates anywhere, the [`route_greedy`] fallback
///   also stuck, or the final validation pass on a complete assignment
///   failing) is reported as [`Conflict::Unroutable`].
pub fn route(
    config: &Config,
    db: &Database,
) -> Result<BTreeMap<(String, String), Pin>, Vec<Conflict>> {
    let (open_roles, mut occupied) = extract_open_roles(config, db)?;

    if open_roles.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut assignment = BTreeMap::new();
    let mut steps: u64 = 0;
    match dfs(
        &open_roles,
        0,
        &mut occupied,
        &mut assignment,
        &mut steps,
        db,
    ) {
        DfsOutcome::Solved => {}
        DfsOutcome::BudgetExceeded => {
            // Fall back to one-shot greedy: same sorted candidate order, no
            // undo. Re-extract fresh state since `dfs` may have left
            // `occupied`/`assignment` mid-mutation from its last (undone)
            // attempt.
            return route_greedy(config, db);
        }
        DfsOutcome::Exhausted(_depth, stuck) => {
            let reason = stuck.reason();
            return Err(vec![Conflict::Unroutable {
                node: stuck.role_node,
                reason,
            }]);
        }
    }

    finalize(config, db, assignment)
}

/// One-shot greedy fallback: the same sorted candidate order [`route`] uses,
/// but takes the first free candidate per role with no backtracking. Used
/// when the backtracking search exceeds [`BACKTRACK_BUDGET`], and directly
/// callable so tests can exercise the no-undo strategy in isolation (without
/// needing to actually drive a search to the budget).
pub fn route_greedy(
    config: &Config,
    db: &Database,
) -> Result<BTreeMap<(String, String), Pin>, Vec<Conflict>> {
    let (open_roles, mut occupied) = extract_open_roles(config, db)?;

    if open_roles.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut assignment = BTreeMap::new();
    for role in &open_roles {
        let free = role.candidates.iter().find(|p| !occupied.contains_key(p));
        match free {
            Some(&pin) => {
                occupied.insert(pin, role.node());
                assignment.insert((role.instance.clone(), role.key.to_string()), pin);
            }
            None => {
                let stuck = stuck_at(role, db, &occupied);
                let reason = stuck.reason();
                return Err(vec![Conflict::Unroutable {
                    node: stuck.role_node,
                    reason,
                }]);
            }
        }
    }

    finalize(config, db, assignment)
}

/// Merge a complete candidate assignment into a synthetic fully-pinned
/// [`Config`] and re-run [`solver::solve`]'s clock/DMA/IRQ/pin-collision
/// checks. A route is only reported successful if this passes, so a
/// successful [`route`] is valid by construction. Any conflict here is
/// reported as [`Conflict::Unroutable`], naming the first such conflict's
/// node and quoting its message as the reason (no information from the
/// underlying check is lost, but the router's external failure contract
/// stays uniform).
fn finalize(
    config: &Config,
    db: &Database,
    assignment: BTreeMap<(String, String), Pin>,
) -> Result<BTreeMap<(String, String), Pin>, Vec<Conflict>> {
    let mut synthetic = config.clone();
    for ((instance, key), pin) in &assignment {
        if let Some(table) = synthetic.peripherals.get_mut(instance) {
            table
                .0
                .insert(key.clone(), toml::Value::String(pin.to_string()));
        }
    }

    let conflicts = solver::solve(&synthetic, db);
    if let Some(first) = conflicts.first() {
        let node = conflict_node(first);
        return Err(vec![Conflict::Unroutable {
            node,
            reason: format!("the solved assignment fails validation: {first}"),
        }]);
    }

    Ok(assignment)
}

/// The node name to attribute an arbitrary [`Conflict`] to, for
/// [`Conflict::Unroutable`]'s `node` field when wrapping a final-validation
/// failure. Every variant already carries something node-shaped; this picks
/// the most relevant one per variant, mirroring how [`crate::solver::Conflict`]
/// itself names nodes for [`Conflict::ClockConstraint`]/[`Conflict::IrqConflict`].
fn conflict_node(c: &Conflict) -> String {
    match c {
        Conflict::PinCollision { pin, .. } => pin.to_string(),
        Conflict::AfMismatch { peripheral, .. }
        | Conflict::InvalidPin { peripheral, .. }
        | Conflict::MissingPin { peripheral, .. }
        | Conflict::ClockDomainDisabled { peripheral, .. }
        | Conflict::PeripheralUnavailable { peripheral, .. } => peripheral.clone(),
        Conflict::ClockConstraint { node, .. }
        | Conflict::DmaCollision { first: node, .. }
        | Conflict::IrqConflict { node, .. }
        | Conflict::Unroutable { node, .. } => node.clone(),
    }
}

/// The outcome of the backtracking DFS: solved, ran out of step budget, or
/// fully exhausted (every candidate at every branch dead-ends). `Exhausted`
/// carries the role-list depth the dead-end was recorded at, alongside the
/// [`StuckAt`] payload, so a caller comparing several branches' outcomes can
/// tell which one actually got furthest (see [`dfs`]'s loop).
enum DfsOutcome {
    Solved,
    BudgetExceeded,
    Exhausted(usize, StuckAt),
}

/// Backtracking DFS over `roles[idx..]`. Tries each of `roles[idx]`'s
/// candidates (skipping any pin currently in `occupied`), tentatively
/// committing it to `occupied`/`assignment`, recursing on `idx + 1`, and
/// undoing on dead-end before trying the next candidate.
fn dfs(
    roles: &[OpenRole],
    idx: usize,
    occupied: &mut Occupied,
    assignment: &mut BTreeMap<(String, String), Pin>,
    steps: &mut u64,
    db: &Database,
) -> DfsOutcome {
    if idx == roles.len() {
        return DfsOutcome::Solved;
    }

    let role = &roles[idx];
    // The deepest dead-end seen across every candidate tried for this role:
    // the role furthest into the list that we ever got stuck on is the most
    // informative to blame in the eventual `Conflict::Unroutable` (everything
    // shallower than it had *some* candidate that worked in isolation; it's
    // the combination that ultimately fails).
    let mut deepest: Option<(usize, StuckAt)> = None;
    let mut tried_any = false;

    for &pin in &role.candidates {
        if occupied.contains_key(&pin) {
            continue;
        }
        tried_any = true;

        *steps += 1;
        if *steps > BACKTRACK_BUDGET {
            return DfsOutcome::BudgetExceeded;
        }

        occupied.insert(pin, role.node());
        assignment.insert((role.instance.clone(), role.key.to_string()), pin);

        match dfs(roles, idx + 1, occupied, assignment, steps, db) {
            DfsOutcome::Solved => return DfsOutcome::Solved,
            DfsOutcome::BudgetExceeded => return DfsOutcome::BudgetExceeded,
            DfsOutcome::Exhausted(depth, stuck) => {
                if deepest.as_ref().is_none_or(|(d, _)| depth > *d) {
                    deepest = Some((depth, stuck));
                }
            }
        }

        occupied.remove(&pin);
        assignment.remove(&(role.instance.clone(), role.key.to_string()));
    }

    if !tried_any {
        // Every candidate for this role (if any existed at all) was already
        // occupied — this role itself is the deepest point of failure, unless
        // a sibling candidate already tried at this same level got deeper
        // (impossible here since `tried_any` is false: no candidate was even
        // attempted, so `deepest` must be `None` too).
        return DfsOutcome::Exhausted(idx, stuck_at(role, db, occupied));
    }

    match deepest {
        Some((depth, stuck)) => DfsOutcome::Exhausted(depth, stuck),
        None => {
            // Unreachable in practice: `tried_any` is true (at least one
            // candidate was attempted) but recursion never returned
            // `Exhausted` either — every attempted branch would have had to
            // return `Solved`/`BudgetExceeded` instead, both of which already
            // return early above. Fall back to blaming this role rather than
            // panicking.
            DfsOutcome::Exhausted(idx, stuck_at(role, db, occupied))
        }
    }
}

/// Build a [`StuckAt`] for `role`, describing **every** candidate the
/// database models for its signal (re-derived via [`Database::candidate_pins`]
/// rather than `role.candidates`, which has already been filtered down to
/// "still free" and would otherwise have nothing left to show) and, for each,
/// who already holds it per the live `occupied` map.
fn stuck_at(role: &OpenRole, db: &Database, occupied: &Occupied) -> StuckAt {
    let all_candidates = db.candidate_pins(&role.peripheral, role.signal);
    let candidates_considered = all_candidates
        .into_iter()
        .map(|pin| (pin, occupied.get(&pin).cloned()))
        .collect();
    StuckAt {
        role_node: role.node(),
        candidates_considered,
    }
}

/// Step 1: extract the open (unrouted) required roles from `config`, in the
/// same lexical/per-role order [`crate::solver::solve`] visits them, and the
/// set of pins already occupied (by already-pinned roles and `[[exti]]`
/// entries). A pre-existing hard conflict in an already-pinned role aborts
/// immediately, reusing the existing [`Conflict`] variant directly.
fn extract_open_roles(
    config: &Config,
    db: &Database,
) -> Result<(Vec<OpenRole>, Occupied), Vec<Conflict>> {
    let mut occupied: Occupied = BTreeMap::new();

    // `[[exti]]` entries are pre-occupied pins, like any other already-pinned
    // role. Unparsable entries are not this module's concern to report (the
    // IRQ verifier already does, in the final validation pass) — skip
    // silently rather than duplicating that diagnostic.
    for entry in &config.exti {
        if let Ok(pin) = Pin::from_str(&entry.pin) {
            occupied.insert(pin, "[[exti]]".to_string());
        }
    }

    let mut raw_open: Vec<(String, String, &'static [Role])> = Vec::new();

    for (instance, table) in &config.peripherals {
        let Some(roles) = model::roles_for(instance) else {
            continue;
        };
        let peripheral = model::peripheral_name(instance);

        if !db.has_peripheral(&peripheral) {
            return Err(vec![Conflict::PeripheralUnavailable {
                peripheral: peripheral.clone(),
                family: config.device.family.clone(),
            }]);
        }

        if let Some(bus) = model::peripheral_bus(&peripheral) {
            let enabled = match bus {
                model::Bus::Ahb1 => config.clocks.ahb1,
                model::Bus::Apb1 => config.clocks.apb1,
                model::Bus::Apb2 => config.clocks.apb2,
            };
            if !enabled {
                return Err(vec![Conflict::ClockDomainDisabled {
                    peripheral: peripheral.clone(),
                    bus,
                }]);
            }
        }

        let mut any_open = false;
        for role in roles {
            match table.pin_str(role.key) {
                None => {
                    if role.required {
                        any_open = true;
                    }
                    // An absent optional role is left alone: never routed,
                    // never flagged (matches `solve()`'s own discipline).
                }
                Some(value) => {
                    let Ok(pin) = Pin::from_str(value) else {
                        return Err(vec![Conflict::InvalidPin {
                            peripheral: peripheral.clone(),
                            key: role.key.to_string(),
                            value: value.to_string(),
                        }]);
                    };
                    if db.find_af(pin, &peripheral, role.signal).is_none() {
                        return Err(vec![Conflict::AfMismatch {
                            pin,
                            peripheral: peripheral.clone(),
                            signal: role.signal.to_string(),
                        }]);
                    }
                    occupied.insert(pin, format!("{instance}.{}", role.key));
                }
            }
        }
        if any_open {
            raw_open.push((instance.clone(), peripheral, roles));
        }
    }

    // Build the actual per-role candidate lists, instance by instance, so
    // `preferred_port` can see every open role of one instance together.
    let mut open_roles: Vec<OpenRole> = Vec::new();
    for (instance, peripheral, roles) in &raw_open {
        let this_instance_open: Vec<&Role> = roles
            .iter()
            .filter(|r| r.required && config.peripherals[instance].pin_str(r.key).is_none())
            .collect();

        let preferred = preferred_port(db, peripheral, &this_instance_open, &occupied);

        for role in this_instance_open {
            let candidates = sorted_candidates(db, peripheral, role.signal, &occupied, preferred);
            open_roles.push(OpenRole {
                instance: instance.clone(),
                peripheral: peripheral.clone(),
                key: role.key,
                signal: role.signal,
                candidates,
            });
        }
    }

    Ok((open_roles, occupied))
}

/// Whether one GPIO port has a free candidate for *every* role in
/// `open_roles` (all belonging to the same instance), given the pins already
/// in `occupied`. If one or more ports qualify, returns the one with the
/// lowest total demand across the instance's roles (ties broken by port
/// letter) — the port the whole-instance assignment should prefer. `None`
/// when no single port covers every role, or the instance has fewer than two
/// open roles (no sibling to share a port with).
fn preferred_port(
    db: &Database,
    peripheral: &str,
    open_roles: &[&Role],
    occupied: &Occupied,
) -> Option<Port> {
    if open_roles.len() < 2 {
        return None;
    }

    let mut best: Option<(Port, usize)> = None;
    for port in [
        Port::A,
        Port::B,
        Port::C,
        Port::D,
        Port::E,
        Port::F,
        Port::G,
        Port::H,
    ] {
        let mut total_demand = 0usize;
        let mut covers_all = true;
        for role in open_roles {
            let candidate_on_port = db
                .candidate_pins(peripheral, role.signal)
                .into_iter()
                .find(|p| p.port == port && !occupied.contains_key(p));
            match candidate_on_port {
                Some(pin) => total_demand += db.alt_functions(pin).count(),
                None => {
                    covers_all = false;
                    break;
                }
            }
        }
        if covers_all {
            best = Some(match best {
                Some((prev_port, prev_demand)) if prev_demand <= total_demand => {
                    (prev_port, prev_demand)
                }
                _ => (port, total_demand),
            });
        }
    }
    best.map(|(port, _)| port)
}

/// The sorted candidate list for one open role: every candidate pin from
/// [`Database::candidate_pins`] minus `occupied`, ordered by the strict
/// lexicographic cost key — ascending pin-demand, then the (no-op) DMA term,
/// then whether the candidate is on `preferred` (same-port-as-sibling). Pin
/// ordinal is the final, deterministic tiebreak (via `Pin`'s `Ord`, already
/// satisfied by sorting a `Vec` of structurally comparable tuples).
fn sorted_candidates(
    db: &Database,
    peripheral: &str,
    signal: &str,
    occupied: &Occupied,
    preferred: Option<Port>,
) -> Vec<Pin> {
    let mut candidates: Vec<Pin> = db
        .candidate_pins(peripheral, signal)
        .into_iter()
        .filter(|p| !occupied.contains_key(p))
        .collect();

    candidates.sort_by_key(|&pin| {
        let demand = db.alt_functions(pin).count();
        // DMA-pressure: always 0. See the module doc comment for why this can
        // never break a tie in the pins-only scope M4 ships — left as an
        // explicit, honest no-op term rather than silently dropped from the
        // key tuple.
        let dma_pressure = 0u8;
        // `false` sorts before `true`, so a candidate on the preferred port
        // (this is `false`) is tried before one that isn't (`true`).
        let off_preferred_port = match preferred {
            Some(p) => pin.port != p,
            None => false,
        };
        (demand, dma_pressure, off_preferred_port, pin)
    });

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn db() -> Database {
        Database::f446re()
    }

    fn parse(text: &str) -> Config {
        config::parse(text).unwrap()
    }

    // --- Simple successful route -----------------------------------------

    #[test]
    fn simple_route_fills_required_roles() {
        // USART2_TX has exactly one candidate (PA2) on the F446; USART2_RX
        // exactly one (PA3). A clean, trivial route.
        let cfg = parse("[peripherals.usart2]\n");
        let result = route(&cfg, &db()).expect("should route");

        assert_eq!(
            result.get(&("usart2".to_string(), "tx".to_string())),
            Some(&Pin::from_str("PA2").unwrap())
        );
        assert_eq!(
            result.get(&("usart2".to_string(), "rx".to_string())),
            Some(&Pin::from_str("PA3").unwrap())
        );
        // Only required roles are routed; usart2 has no pin set for the
        // optional cts/rts/ck roles, and none should appear in the result.
        assert_eq!(result.len(), 2, "got {result:?}");
    }

    #[test]
    fn already_pinned_roles_are_not_in_the_result() {
        // tx is already pinned by the user; only rx should appear in the
        // routed result.
        let cfg = parse("[peripherals.usart2]\ntx = \"PA2\"\n");
        let result = route(&cfg, &db()).expect("should route");

        assert!(
            !result.contains_key(&("usart2".to_string(), "tx".to_string())),
            "got {result:?}"
        );
        assert_eq!(
            result.get(&("usart2".to_string(), "rx".to_string())),
            Some(&Pin::from_str("PA3").unwrap())
        );
        assert_eq!(result.len(), 1, "got {result:?}");
    }

    #[test]
    fn fully_pinned_instance_has_no_open_roles() {
        let cfg = parse("[peripherals.usart2]\ntx = \"PA2\"\nrx = \"PA3\"\n");
        let result = route(&cfg, &db()).expect("should route");
        assert_eq!(result, std::collections::BTreeMap::new());
    }

    #[test]
    fn optional_roles_are_never_auto_routed() {
        // SPI1 has no `nss` set (optional); route() must fill mosi/miso/sck
        // but never invent an nss pin.
        let cfg = parse("[peripherals.spi1]\n");
        let result = route(&cfg, &db()).expect("should route");

        assert!(
            !result.contains_key(&("spi1".to_string(), "nss".to_string())),
            "got {result:?}"
        );
        assert!(result.contains_key(&("spi1".to_string(), "mosi".to_string())));
        assert!(result.contains_key(&("spi1".to_string(), "miso".to_string())));
        assert!(result.contains_key(&("spi1".to_string(), "sck".to_string())));
    }

    #[test]
    fn routed_assignment_passes_a_passthrough_check() {
        // Issue #20's explicit "passthrough" criterion: merging the routed
        // assignment back into the config and re-checking it must be clean.
        let cfg = parse("[peripherals.usart2]\n\n[peripherals.spi1]\n");
        let result = route(&cfg, &db()).expect("should route");

        let mut synthetic = cfg.clone();
        for ((instance, key), pin) in &result {
            synthetic
                .peripherals
                .get_mut(instance)
                .unwrap()
                .0
                .insert(key.clone(), toml::Value::String(pin.to_string()));
        }
        let conflicts = solver::solve(&synthetic, &db());
        assert_eq!(conflicts, vec![], "got {conflicts:?}");
    }

    // --- Pre-existing hard conflict aborts before search starts -----------

    #[test]
    fn invalid_pin_on_an_already_pinned_role_aborts() {
        let cfg = parse("[peripherals.usart2]\ntx = \"PZ9\"\n");
        let err = route(&cfg, &db()).expect_err("should abort, not route around it");

        assert_eq!(err.len(), 1, "got {err:?}");
        assert!(
            matches!(&err[0], Conflict::InvalidPin { value, .. } if value == "PZ9"),
            "got {err:?}"
        );
    }

    #[test]
    fn af_mismatch_on_an_already_pinned_role_aborts() {
        // PB0 does not carry USART2_TX on the F446.
        let cfg = parse("[peripherals.usart2]\ntx = \"PB0\"\n");
        let err = route(&cfg, &db()).expect_err("should abort");

        assert_eq!(err.len(), 1, "got {err:?}");
        assert!(
            matches!(
                &err[0],
                Conflict::AfMismatch { pin, signal, .. }
                    if pin.to_string() == "PB0" && signal == "TX"
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn unavailable_peripheral_aborts_before_search() {
        let cfg = parse("[device]\nfamily = \"STM32F411RE\"\n\n[peripherals.uart4]\n");
        let err = route(&cfg, &Database::f411re()).expect_err("should abort");

        assert_eq!(err.len(), 1, "got {err:?}");
        assert!(
            matches!(
                &err[0],
                Conflict::PeripheralUnavailable { peripheral, family }
                    if peripheral == "UART4" && family == "STM32F411RE"
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn disabled_clock_domain_aborts_before_search() {
        // SPI1 lives on APB2; disabling it must abort before any search.
        let cfg = parse("[clocks]\napb2 = false\n\n[peripherals.spi1]\n");
        let err = route(&cfg, &db()).expect_err("should abort");

        assert_eq!(err.len(), 1, "got {err:?}");
        assert!(
            matches!(
                &err[0],
                Conflict::ClockDomainDisabled { peripheral, .. } if peripheral == "SPI1"
            ),
            "got {err:?}"
        );
    }

    // --- Over-constrained -> Unroutable ------------------------------------

    #[test]
    fn over_constrained_role_is_unroutable() {
        // USART2_TX's only candidate is PA2. Pre-occupy PA2 with an unrelated
        // peripheral signal so TX has zero free candidates.
        let cfg = parse("[peripherals.tim5]\nchannel3 = \"PA2\"\n\n[peripherals.usart2]\n");
        let err = route(&cfg, &db()).expect_err("should be unroutable");

        assert_eq!(err.len(), 1, "got {err:?}");
        match &err[0] {
            Conflict::Unroutable { node, reason } => {
                assert_eq!(node, "USART2_TX");
                assert!(reason.contains("PA2"), "reason: {reason}");
            }
            other => panic!("expected Unroutable, got {other:?}"),
        }
    }

    #[test]
    fn exti_pre_occupied_pin_makes_a_role_unroutable() {
        // `[[exti]]` entries are pre-occupied pins, exactly like any other
        // already-pinned role. USART2_TX's only candidate is PA2; claiming it
        // via `[[exti]]` (not via another peripheral) must still make TX
        // unroutable.
        let cfg = parse("[[exti]]\npin = \"PA2\"\n\n[peripherals.usart2]\n");
        let err = route(&cfg, &db()).expect_err("should be unroutable");

        assert_eq!(err.len(), 1, "got {err:?}");
        assert!(matches!(&err[0], Conflict::Unroutable { node, .. } if node == "USART2_TX"));
    }

    // --- Same-port preference ----------------------------------------------

    #[test]
    fn same_port_preference_picks_single_port_solution() {
        // SPI1 MOSI=[PA7(demand 7), PB5(10)], MISO=[PA6(7), PB4(7)],
        // SCK=[PA5(6), PB3(7)]. MOSI and SCK already win on demand alone
        // (port A); MISO is an exact demand tie, broken only by the
        // same-port-as-sibling preference, which must pick PA6 (port A) over
        // the equally-cheap PB4 (port B) so the whole instance lands on one
        // port.
        let cfg = parse("[peripherals.spi1]\n");
        let result = route(&cfg, &db()).expect("should route");

        let mosi = result[&("spi1".to_string(), "mosi".to_string())];
        let miso = result[&("spi1".to_string(), "miso".to_string())];
        let sck = result[&("spi1".to_string(), "sck".to_string())];

        assert_eq!(mosi, Pin::from_str("PA7").unwrap());
        assert_eq!(
            miso,
            Pin::from_str("PA6").unwrap(),
            "MISO should prefer port A to match its siblings, got {miso}"
        );
        assert_eq!(sck, Pin::from_str("PA5").unwrap());
        assert_eq!(mosi.port, miso.port);
        assert_eq!(miso.port, sck.port);
    }

    // --- Demand heuristic ---------------------------------------------------

    #[test]
    fn demand_heuristic_prefers_lower_demand_pin() {
        // SPI1 MOSI's two candidates are PA7 (demand 7) and PB5 (demand 10);
        // the router must pick the lower-demand PA7.
        let cfg = parse("[peripherals.spi1]\nmiso = \"PA6\"\nsck = \"PA5\"\n");
        let result = route(&cfg, &db()).expect("should route");

        assert_eq!(
            result[&("spi1".to_string(), "mosi".to_string())],
            Pin::from_str("PA7").unwrap()
        );
    }

    // --- Actual backtracking required ---------------------------------------

    #[test]
    fn backtracking_required_when_first_choice_dead_ends() {
        // USART1_TX's two candidates are PA9 (demand 7) and PB6 (demand 8);
        // `[[exti]] pin = "PA9"` removes PA9, leaving TX exactly one
        // candidate: PB6. I2C1_SCL's two candidates, PB6 and PB8, are an
        // exact demand tie (both 8) that also tie on the same-port
        // preference (both already port B) -- the deciding tiebreak is pin
        // ordinal, which a non-backtracking strategy would resolve to PB6
        // (the lexically-first one) every time, since nothing in I2C1's own
        // candidate evaluation can see that USART1_TX needs it. Greedy
        // commits I2C1_SCL -> PB6 first (lexical instance order: "i2c1" <
        // "usart1"), which starves USART1_TX of its only remaining
        // candidate -- proven below by `route_greedy` actually failing on
        // this exact fixture. Only a search that backtracks I2C1_SCL to PB8
        // -- freeing PB6 for USART1_TX -- finds a complete assignment.
        let cfg = parse("[[exti]]\npin = \"PA9\"\n\n[peripherals.i2c1]\n\n[peripherals.usart1]\n");

        // Confirm the premise: the non-backtracking strategy really does
        // fail on this fixture (otherwise this wouldn't be testing
        // backtracking at all).
        assert!(
            route_greedy(&cfg, &db()).is_err(),
            "fixture premise violated: greedy should fail here"
        );

        let result = route(&cfg, &db()).expect("backtracking should find a complete assignment");

        assert_eq!(
            result[&("usart1".to_string(), "tx".to_string())],
            Pin::from_str("PB6").unwrap()
        );
        assert_eq!(
            result[&("i2c1".to_string(), "scl".to_string())],
            Pin::from_str("PB8").unwrap(),
            "I2C1_SCL must have backtracked off PB6 to free it for USART1_TX"
        );
        // The full assignment must also be internally consistent (no pin
        // used twice) and pass the final validation pass implicitly, since
        // `route` only returns `Ok` after that check.
        let mut pins: Vec<Pin> = result.values().copied().collect();
        pins.sort_unstable();
        let mut deduped = pins.clone();
        deduped.dedup();
        assert_eq!(pins, deduped, "no pin should be assigned twice: {result:?}");
    }

    // --- Greedy fallback tested directly in isolation -----------------------

    #[test]
    fn greedy_fallback_takes_first_choice_with_no_undo() {
        // Same fixture as the backtracking test above, called directly
        // through `route_greedy` rather than via `route`'s budget-exceeded
        // path: confirms the no-undo strategy is independently reachable and
        // behaves exactly as documented (first sorted candidate per role, no
        // backtracking), without needing to actually drive a search to the
        // 100,000-step budget.
        let cfg = parse("[[exti]]\npin = \"PA9\"\n\n[peripherals.i2c1]\n\n[peripherals.usart1]\n");

        let err = route_greedy(&cfg, &db()).expect_err("greedy should get stuck on this fixture");

        assert_eq!(err.len(), 1, "got {err:?}");
        match &err[0] {
            Conflict::Unroutable { node, reason } => {
                assert_eq!(node, "USART1_TX");
                // Both of TX's real candidates are named, and what holds
                // each: PA9 by the EXTI entry, PB6 by I2C1_SCL's
                // un-backtracked first choice.
                assert!(reason.contains("PA9"), "reason: {reason}");
                assert!(reason.contains("PB6"), "reason: {reason}");
                assert!(reason.contains("exti"), "reason: {reason}");
                assert!(reason.contains("I2C1_SCL"), "reason: {reason}");
            }
            other => panic!("expected Unroutable, got {other:?}"),
        }
    }

    #[test]
    fn greedy_fallback_succeeds_on_an_uncontested_config() {
        // Sanity check that route_greedy works standalone (not just as a
        // budget-exceeded fallback) on a fixture with no backtracking-forcing
        // contention.
        let cfg = parse("[peripherals.usart2]\n");
        let result = route_greedy(&cfg, &db()).expect("should route");

        assert_eq!(
            result.get(&("usart2".to_string(), "tx".to_string())),
            Some(&Pin::from_str("PA2").unwrap())
        );
        assert_eq!(
            result.get(&("usart2".to_string(), "rx".to_string())),
            Some(&Pin::from_str("PA3").unwrap())
        );
    }
}
