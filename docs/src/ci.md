# CI integration

Because `nucleus check` exits non-zero on any conflict, validating your
`stm32.toml` in CI is a one-liner. Nucleus ships a reusable composite action
that installs the CLI, runs `check` (and optionally `build`), and posts a PR
summary with the conflict count and firmware size.

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
          # build: "true"   # also compile firmware (needs the ARM toolchain)
```

`nucleus init` scaffolds an equivalent workflow for you.

## Action inputs

| Input | Default | Description |
|---|---|---|
| `config` | `stm32.toml` | Path to the config to validate. |
| `build` | `false` | Also run `nucleus build` (requires `arm-none-eabi-gcc` + `cmake` in the runner). |
| `version` | `*` | `nucleus-cli` version to install from crates.io, or `git` to build from `main`. |
| `comment` | `true` | Post the summary as a PR comment (needs `pull-requests: write`). |

## Action outputs

| Output | Description |
|---|---|
| `conflicts` | Number of conflicts `nucleus check` reported. |
| `firmware-size` | Size of `build/firmware.bin` in bytes (when `build: true`). |

The action also writes the summary to the workflow run's **Job Summary**, so it
is visible even without comment permissions.

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
