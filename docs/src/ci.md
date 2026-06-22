# CI integration

Because `nucleus check` exits non-zero on any conflict, validating your
`stm32.toml` in CI is a one-liner. Nucleus ships a reusable composite action
that installs the CLI, runs `check` → `build` → `test` (QEMU always,
hardware optionally), and posts a PR summary with per-backend results — see
[Test History & CI](test-history.md) for the full per-backend report format.

## Quick start: copy-paste `nucleus.yml`

Drop this into `.github/workflows/nucleus.yml`:

```yaml
name: nucleus
on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read
  pull-requests: write   # needed to post the PR summary comment

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: harshverma27/nucleus/.github/actions/nucleus@main
        with:
          config: stm32.toml
          build: "true"      # also compile firmware (needs the ARM toolchain)
          # run_tests defaults to "true" once build is on: runs `nucleus test --backend qemu`
          # hardware: "true"  # also run the hardware leg (self-hosted runner + board only)
```

`nucleus init` scaffolds an equivalent workflow for you.

## Action inputs

| Input | Default | Description |
|---|---|---|
| `config` | `stm32.toml` | Path to the config to validate. |
| `build` | `false` | Also run `nucleus build` (requires `arm-none-eabi-gcc` + `cmake` in the runner). |
| `run_tests` | `true` | Run `nucleus test` after a successful build (QEMU always; hardware only if `hardware: true`). Requires `build: true`. |
| `hardware` | `false` | Also run the hardware test leg (needs a self-hosted runner with a connected board + OpenOCD). Off by default — recorded as **skipped**, never failed, so a hosted runner with no board stays green. |
| `version` | `*` | `nucleus-cli` version to install from crates.io, or `git` to build from `main`. |
| `comment` | `true` | Post the summary as a PR comment (needs `pull-requests: write`). |

## Action outputs

| Output | Description |
|---|---|
| `conflicts` | Number of conflicts `nucleus check` reported. |
| `firmware-size` | Size of `build/firmware.bin` in bytes (when `build: true`). |
| `qemu-passed` | Assertions passed on the QEMU backend (empty if tests didn't run). |
| `hardware-passed` | Assertions passed on the hardware backend (empty if the leg was skipped). |

The action also writes the summary to the workflow run's **Job Summary**, so it
is visible even without comment permissions, and uploads
`tests/test_history.json` as the `nucleus-test-history` artifact whenever
tests ran.

> `version` defaults to `*` (the latest crates.io release). Pin a specific
> release (e.g. `version: 0.1.0`) for reproducible CI, or set `version: git` to
> build the CLI from `main` instead of crates.io.

## Doing it by hand

If you'd rather not use the action:

```yaml
- uses: dtolnay/rust-toolchain@stable
- run: cargo install nucleus-cli --locked
- run: nucleus check
```
