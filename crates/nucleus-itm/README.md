# nucleus-itm

A hand-rolled ARM CoreSight **ITM/SWO** packet decoder.

## Status (Phase 5 — complete)

`Decoder::decode(&[u8]) -> Vec<Packet>` is a streaming decoder for the SWO byte
stream (ARMv7-M ARM, DDI 0403E §C1.10):

- **Streaming** — fed arbitrary chunks; packets may span boundaries, with the
  partial tail buffered until the rest arrives.
- **Never panics** — zero dependencies on the parsing path, every slice access
  length-checked, unrecognized protocol headers skipped to resync.
- **Resynchronizes** — synchronization packets re-align after a dropped
  connection; the internal buffer is capped for O(1) memory.

Decoded `Packet`s: `Instrumentation` (SWIT software source — ports 0–31, 1/2/4
byte), `Hardware` (DWT), `Overflow`, `Synchronization`, `LocalTimestamp`,
`GlobalTimestamp`, `Extension`. The decoder is config-agnostic; naming/typing of
ports is the caller's job (see `nucleus-trace`).

## Robustness

A randomized test (deterministic xorshift PRNG) feeds thousands of arbitrary
byte streams in arbitrary chunk sizes and asserts (a) no panic and (b) that
chunking never changes the decoded packet sequence — the README's "zero panics
under fuzzing" requirement. A `cargo fuzz` target can wrap `Decoder::decode`
directly for deeper coverage.
