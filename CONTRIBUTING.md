# Contributing to Nucleus

Thanks for your interest in Nucleus! This guide covers how to build, test, and
submit changes.

## Project shape

Nucleus is a Cargo workspace (the Rust CLI + libraries) plus a thin VS Code
extension under `extension/`. Read [`CLAUDE.md`](CLAUDE.md) and [`README.md`](README.md)
first — they define the component boundaries and the binding architectural
rules. The two that trip people up most:

1. **The extension contains zero business logic.** Constraint checking,
   decoding, and validation live in the Rust crates, never in TypeScript.
2. **Generated code only calls stock ST HAL `Init` functions** — the codegen
   never reimplements the HAL.

## Prerequisites

- **Rust** ≥ 1.85 (the workspace MSRV). Install via [rustup](https://rustup.rs).
- For the extension: **Node.js** ≥ 18 and npm.
- Optional, only for `nucleus build`/`flash`: `arm-none-eabi-gcc`, `cmake`,
  `st-flash`, and a NUCLEO-F446RE board.

## The local gate

`make check` runs the exact checks CI runs. **Run it before every push:**

```sh
make check        # = fmt-check + clippy (-D warnings) + test
```

Individual steps:

```sh
make fmt          # apply rustfmt
make fmt-check    # verify formatting
make lint         # clippy with warnings denied
make test         # cargo test --workspace
```

For the extension:

```sh
cd extension
npm install
npm run typecheck   # tsc --noEmit
npm run build       # esbuild bundle
```

## Working on a change

1. Branch off `main` (e.g. `git checkout -b fix-spi-nss`).
2. Make the change **with tests** — every crate is independently testable, and
   new behaviour should come with a unit or integration test.
3. Keep changes surgical and match the surrounding style (comment density,
   naming, idioms).
4. Run `make check` until green.
5. Open a PR. CI must pass before merge.

### Testing notes

- **`nucleus-db`** output must stay byte-deterministic; a test cross-validates
  the generated table against a hand-verified datasheet seed.
- **`nucleus-itm`** must never panic. New decoding paths must keep the
  randomized/fuzz test green; consider adding a `cargo fuzz` case for tricky
  framing.
- Upstream pack-data quirks are **never** fixed by editing `packdata/` — they go
  in the patch table in `nucleus-db/src/pack.rs`.

## Commit and PR conventions

- Write focused commits with a clear subject line and a body explaining *why*.
- Reference the roadmap phase where relevant (see `README.md`).
- PRs should describe what changed, how it was verified, and any follow-ups.

## Licensing

Nucleus is dual-licensed under **MIT OR Apache-2.0**. By contributing, you agree
that your contributions are licensed under the same terms (see `LICENSE-MIT` and
`LICENSE-APACHE`). No CLA is required.

## Reporting bugs / proposing features

Open an issue with: what you expected, what happened, and a minimal repro
(ideally a small `stm32.toml` or byte capture). For the trace decoder, a raw
`.swo` capture that reproduces the problem is invaluable.
