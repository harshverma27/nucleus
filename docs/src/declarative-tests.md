# Declarative Tests

M6 lets you state a hardware behavior as one line in `stm32.toml` instead of
writing a test harness: `[[test]]` blocks with an `assertion` string, parsed
into a typed `Assertion` and evaluated against the [HIL
backend](hil-backends.md) trait — no host code to write for the common
cases (a pin toggling, a UART echoing, a trace event firing).

## `[[test]]` schema

```toml
[[test]]
name = "uart_echo"                            # required
assertion = "USART2 echoes \"ping\" within 10ms"
timeout_ms = 100                              # optional, default 1000
backend = "both"                              # optional: "qemu" | "hardware" | "both" (default)
type = "declarative"                          # optional, default — see scripted-tests.md for "scripted"
```

`assertion` and the other fields are independently optional in the parser,
but an entry with no `assertion` and no `script` is meaningless — fill at
least one.

## The assertion grammar (`nucleus-compiler/src/assertion.rs`)

Four forms, hand-parsed (no parser-combinator dependency, consistent with the
rest of the config parser):

| Form | Produces |
|---|---|
| `pin <PIN> toggles at <N>Hz ±<N>%` | `Assertion::PinToggles { pin, hz, tolerance_pct }` |
| `pin <PIN> is <high\|low> within <N>ms` | `Assertion::PinState { pin, level, within }` |
| `<PERIPH> echoes "<text>" within <N>ms` | `Assertion::UartEcho { instance, payload, within }` |
| `trace event "<pattern>" within <N>ms` | `Assertion::ItmEvent { pattern, within }` |

This is the *syntactic* layer only — `pin` and `instance` are kept as raw
strings here; resolving them against the `nucleus-db` pin/peripheral tables
and actually sampling the backend happens when `nucleus test` runs the
assertion. A malformed assertion string is a parse error, reported the same
way a malformed TOML value is.

Examples of each form:

```toml
assertion = "pin PA5 toggles at 1Hz ±5%"
assertion = "pin PC13 is low within 10ms"
assertion = "USART2 echoes \"ping\" within 10ms"
assertion = "trace event \"boot complete\" within 500ms"
```

## Evaluating against a backend

`nucleus test` resolves each assertion against whichever [`Backend`](hil-backends.md)
is selected:

- `PinToggles`/`PinState` → repeated `Backend::pin()` samples.
- `UartEcho` → `Backend::write_mem32`/`read_mem32` against the peripheral's
  data register, or (on hardware) a UART transaction over the same channel
  `nucleus-trace` uses.
- `ItmEvent` → `Backend::await_itm_event()`, matching the decoded packet
  against `pattern`.

Remember the [QEMU fidelity gap](hil-backends.md#two-backends-one-fidelity-gap):
QEMU's `netduinoplus2` machine has no real GPIO model, so a `PinToggles`/
`PinState` assertion only meaningfully verifies on the hardware backend —
it still runs on QEMU (nothing special-cases it), but will read back `0`
there. UART and trace-event assertions are fully verified on both.

## Running

```sh
nucleus test                      # all backends, all [[test]] blocks in ./stm32.toml
nucleus test --test uart_echo     # just this one
nucleus test --backend qemu       # just this backend
```

See [Dual-Backend HIL Substrate](hil-backends.md) for the full CLI surface,
exit-code rules, and how results land in
[`tests/test_history.json`](test-history.md).

## See also

- [Dual-Backend HIL Substrate](hil-backends.md)
- [Scripted Tests](scripted-tests.md) — for assertions too complex to express as one string
- [Test History & CI](test-history.md)
