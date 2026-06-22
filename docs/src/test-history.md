# Test History & CI

Nucleus v2's history is **deliberately minimal**: an earlier design called
for a content-addressed `.nucleus/` ledger with version hashing and signed
verification reports. That was built, then ripped out — the team decided the
complexity wasn't earning its keep for what's actually needed (a simple
record of what passed) and replaced it with one flat JSON file. If you're
looking for hashing, artifacts, or a version store: there isn't one, on
purpose.

## `tests/test_history.json`

`nucleus test` creates this file on first run, then appends one **run**
every time after. No hashing, no content-addressing — just an append-only
JSON file that travels with the repo (commit it, or don't; it's a normal
file).

```json
{
  "runs": [
    {
      "timestamp": 1750000000,
      "tests": [
        { "name": "uart_echo", "backend": "qemu", "status": "pass", "detail": "echoed within 8ms" },
        { "name": "uart_echo", "backend": "hardware", "status": "skip", "detail": "no probe attached" }
      ]
    }
  ]
}
```

One `TestEntry` per assertion-on-a-backend (so a single `[[test]]` block run
on both backends produces two entries in the same run).

## `nucleus history`

```sh
nucleus history [path]              # human table, oldest run first
nucleus history [path] --last N     # only the N most recent runs
nucleus history [path] --graph      # per-run pass/fail/skip as JSON
```

```sh
$ nucleus history
#1   2025-06-15 15:06:40Z  1 passed, 0 failed, 1 skipped
```

`--graph` is the dashboard's/export data source — always valid JSON, even
against an empty history:

```sh
$ nucleus history --graph
{
  "schema": "nucleus.history.v1",
  "runs": [
    { "timestamp": 1750000000, "pass": 1, "fail": 0, "skip": 1 }
  ]
}
```

## `nucleus show [run]`

Every assertion result for one run — defaults to the latest, or pass the
1-based number `nucleus history` listed.

```sh
$ nucleus show
run      #1
recorded 2025-06-15 15:06:40Z
summary  1 passed, 0 failed, 1 skipped
tests:
    PASS uart_echo [qemu]: echoed within 8ms
    SKIP uart_echo [hardware]: no probe attached
```

## The dashboard's history bar chart

The VS Code extension's React dashboard (see [Trace Dashboard & ITM
Decoding](itm-trace.md)) renders `nucleus history --graph`'s output as a bar
chart — one bar per run, stacked pass/fail/skip. Per the "extension has zero
business logic" rule, all counting happens in `nucleus-history`/Rust; the
TypeScript side only draws what it's given. Trend lines, CPU/timing charts,
and a clickable trend-graph link were all part of the original M9 design and
were descoped along with the ledger — the bar chart is what shipped.

## CI: the composite action

`.github/actions/nucleus/action.yml` wraps `check` → `build` → `test` (QEMU
always, hardware optionally) → a PR summary, with each stage degrading
gracefully if the previous one's inputs are off.

```yaml
- uses: harshverma27/nucleus/.github/actions/nucleus@main
  with:
    config: stm32.toml
    build: "true"        # nucleus build (needs the ARM toolchain)
    run_tests: "true"    # nucleus test, after a successful build (default)
    hardware: "false"    # also run the hardware leg (needs a self-hosted runner + board)
```

| Input | Default | Description |
|---|---|---|
| `config` | `stm32.toml` | Path to the config to validate. |
| `build` | `false` | Also run `nucleus build`. |
| `run_tests` | `true` | Run `nucleus test` after a successful build (QEMU always; hardware only if `hardware: true`). Requires `build: true`. |
| `hardware` | `false` | Also run the hardware test leg. Off by default — the report records it as **skipped**, never failed, so a hosted runner with no board stays green. |
| `version` | `*` | `nucleus-cli` version from crates.io, or `git` to build from `main`. |
| `comment` | `true` | Post the summary as a PR comment. |

Outputs: `conflicts`, `firmware-size`, `qemu-passed`, `hardware-passed`. The
PR comment (and the workflow's Job Summary, visible even without comment
permissions) reports per-backend pass/fail counts and uploads
`tests/test_history.json` as the `nucleus-test-history` artifact. A signed,
cryptographic verification report was part of the original M9 design and was
descoped with the ledger — the plain-text summary + raw history artifact is
what ships.

## See also

- [Dual-Backend HIL Substrate](hil-backends.md)
- [Trace Dashboard & ITM Decoding](itm-trace.md)
