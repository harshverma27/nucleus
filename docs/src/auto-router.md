# Constraint Auto-Router

M1–M3 all *validate* an explicit, fully-pinned `stm32.toml` and report a
conflict when it's wrong. M4 inverts the problem: name a peripheral instance
without assigning some or all of its pins, and `nucleus route` searches the
pin/AF database for a complete, valid, deterministic assignment for exactly
the roles left open — declare intent, get a pinout.

## Scope: pins only

You always name the exact peripheral instance (`usart2`, `spi1`); the router
never picks *which* instance to use, only *which pins*:

- A role with its key **absent** → route this role.
- A role with its key **present** → respect this pin as fixed (validated
  exactly like `nucleus check` would; a pre-existing hard conflict aborts
  routing before search starts).
- An **optional** role left absent → left alone. The router never
  force-allocates a pin for a signal you have no use for.
- `[[exti]]` entries are pre-occupied pins, treated like any other
  already-pinned role — there's no EXTI auto-routing in M4.

## Cost function

Candidate pins for each open role are sorted by a **strict lexicographic
priority** — not a weighted sum, so there are no tuning constants to fight:

1. **Ascending pin-demand** — how many AF-mapping rows reference that pin
   across the whole database. Lower demand first: using an uncontested pin
   now keeps high-demand pins free for later roles.
2. **DMA pressure** — a documented no-op for this milestone. `nucleus-db`'s
   DMA candidate slots key off `(peripheral, direction)` only, with no
   dependency on which pin/AF was chosen, so this tier can never break a tie
   in the pins-only scope M4 ships. Kept as an explicit always-equal
   comparison (rather than silently dropped) so a future instance-selection
   extension has an obvious place to plug in.
3. **Same-port-as-sibling preference** — for a multi-role instance, if one
   GPIO port has a free candidate for *every* still-open role of that
   instance, every role's sort prefers that port. Lowest priority, so it only
   breaks ties demand left open.

## Search

A backtracking depth-first search over the open roles: try the sorted
candidates for the first open role, recurse on the rest, undo and try the
next candidate on dead-end. A global step counter spans the whole search —
past a fixed budget (100,000 steps; a pathological-input safety net, not a
normal-path concern) the search aborts and falls back to a pure greedy
strategy (same sort order, first candidate per role, no undo).

On a complete candidate assignment, the result is merged into a synthetic
fully-pinned config and re-run through the *same* clock/DMA/IRQ/pin-collision
checks `nucleus check` runs — a successful route is valid by construction,
not just by the router's own bookkeeping. Any failure (search exhaustion,
greedy exhaustion, or this final validation pass) is reported as
`Conflict::Unroutable`.

## Usage

```sh
nucleus route [path]              # print the routed config to stdout
nucleus route [path] --out FILE   # write it to FILE instead
```

`path` defaults to `./stm32.toml`. Exit code `0` on a successful route, `1`
if any role couldn't be routed (or on a parse error) — same gating
discipline as `nucleus check`.

## Example: a clean route

```toml
# tests/fixtures/route_simple.toml
[peripherals.usart2]

[peripherals.spi1]
```

```sh
$ nucleus route tests/fixtures/route_simple.toml
[peripherals.usart2]
rx = "PA3"
tx = "PA2"

[peripherals.spi1]
miso = "PA6"
mosi = "PA7"
sck = "PA5"
```

## Example: unroutable

```toml
# tests/fixtures/route_overconstrained.toml — tim5.channel3 pre-occupies PA2,
# USART2_TX's only candidate pin on the F446, leaving zero free candidates.
[peripherals.tim5]
channel3 = "PA2"

[peripherals.usart2]
```

```sh
$ nucleus route tests/fixtures/route_overconstrained.toml
tests/fixtures/route_overconstrained.toml: could not route (1 conflict):

  error: unroutable [USART2_TX]: no free pin among candidates: PA2 (held by tim5.channel3)
```

## See also

- [Pinout & Peripheral Verification](pinout-verification.md)
- [Clock-Tree Solver](clock-tree.md), [DMA Arbitration](dma-arbitration.md), [IRQ / NVIC Verifier](irq-verifier.md)
