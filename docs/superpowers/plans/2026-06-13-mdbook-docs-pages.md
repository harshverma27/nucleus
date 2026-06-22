# mdBook Docs Site, GitHub Pages, and Issue Templates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure `docs/` into an mdBook site (with a new ITM/OpenOCD firmware-integration chapter), publish it to GitHub Pages via a new Actions workflow, add GitHub issue-form templates, and update README/CHANGELOG/CLAUDE.md to reflect Phase 8's remaining exit criteria as done.

**Architecture:** `docs/` becomes an mdBook source tree (`docs/book.toml` + `docs/src/*.md` + `docs/src/SUMMARY.md`), built by `mdbook build docs` into `docs/book/` (gitignored). A new `.github/workflows/docs.yml` builds and deploys that output to GitHub Pages on every push to `main` that touches `docs/`. Existing docs (`installation.md`, `cli.md`, `ci.md`) move into the book mostly unchanged; `docs/README.md` becomes `introduction.md`; `demo/instructions.md` becomes `quickstart.md`; a new `itm-trace.md` documents firmware-side ITM/SWO setup and the matching OpenOCD commands.

**Tech Stack:** mdBook (Rust docs tool, installed via `cargo install`), GitHub Actions (`actions/configure-pages`, `actions/upload-pages-artifact`, `actions/deploy-pages`), GitHub issue forms (YAML).

**Spec:** `docs/superpowers/specs/2026-06-13-mdbook-docs-pages-design.md`

---

### Task 1: Move existing docs into an mdBook `src/` tree

**Files:**
- Create: `docs/book.toml`
- Move: `docs/installation.md` → `docs/src/installation.md`
- Move: `docs/cli.md` → `docs/src/cli.md`
- Move: `docs/ci.md` → `docs/src/ci.md`
- Move: `docs/README.md` → `docs/src/introduction.md`

- [ ] **Step 1: Create the `docs/src/` directory and move the existing chapter files into it**

```bash
mkdir -p docs/src
git mv docs/installation.md docs/src/installation.md
git mv docs/cli.md docs/src/cli.md
git mv docs/ci.md docs/src/ci.md
git mv docs/README.md docs/src/introduction.md
```

- [ ] **Step 2: Create `docs/book.toml`**

```toml
[book]
title = "Nucleus"
description = "A CLI-first STM32 developer platform"
authors = ["Harsh Verma"]
language = "en"
src = "src"

[output.html]
git-repository-url = "https://github.com/harshverma27/nucleus"
edit-url-template = "https://github.com/harshverma27/nucleus/edit/main/docs/{path}"
```

- [ ] **Step 3: Verify the moves**

Run: `git status --porcelain`
Expected: four `R` (renamed) entries for the four moved files, plus one new
untracked file `docs/book.toml`. No files were deleted without a
corresponding move.

- [ ] **Step 4: Commit**

```bash
git add -A docs/
git commit -m "Move docs/ into an mdBook src/ tree"
```

---

### Task 2: Rewrite `docs/src/introduction.md` as the book's landing page

**Files:**
- Modify: `docs/src/introduction.md` (full rewrite)

- [ ] **Step 1: Replace the file contents**

The current file is framed as a temporary index ("until mdBook lands in Phase
8"). Replace its entire contents with:

```markdown
# Introduction

A CLI-first STM32 developer platform: declarative `stm32.toml` → validated HAL
init code → flashed firmware, plus a real-time ITM trace dashboard.

Nucleus replaces STM32CubeIDE/CubeMX's graphical pin configuration and
proprietary debug tooling with a version-controllable, CI-friendly CLI and a
thin VS Code extension.

## At a glance

```toml
# stm32.toml
[device]
family   = "STM32F446RE"
board    = "NUCLEO-F446RE"
clock_hz = 180_000_000

[peripherals.usart2]
tx   = "PA2"
rx   = "PA3"
baud = 115200
```

```sh
nucleus check     # validate against the constraint database
nucleus build     # generate HAL init code + build firmware
nucleus trace     # decode ITM/SWO and stream to the dashboard
```

Nucleus supports two NUCLEO boards out of the box: **NUCLEO-F446RE**
(`STM32F446RE`) and **NUCLEO-F411RE** (`STM32F411RE`) — pick one with
`nucleus init --board <name>`.

## What's in this book

- **[Installation](installation.md)** — install the `nucleus` CLI and the VS
  Code extension.
- **[Quickstart: Blink an LED](quickstart.md)** — from a clean machine to a
  blinking LED on a NUCLEO-F446RE.
- **[CLI Usage](cli.md)** — `check`, `init`, `build`, `flash`, `lsp`, `trace`.
- **[Enabling ITM Trace](itm-trace.md)** — wire up SWO/ITM in firmware and
  OpenOCD for `nucleus trace`.
- **[CI Integration](ci.md)** — gate PRs with `nucleus check` via the reusable
  action.
```

Note: the fenced code blocks above are nested inside the outer markdown fence
shown in this plan step — when creating the actual file, use single (not
escaped) triple-backtick fences for the `toml` and `sh` blocks, exactly as
written.

- [ ] **Step 2: Verify there are no leftover references to the old framing**

Run: `grep -n "Phase 8\|until then\|canonical reference" docs/src/introduction.md`
Expected: no output (empty).

- [ ] **Step 3: Commit**

```bash
git add docs/src/introduction.md
git commit -m "Rewrite docs/src/introduction.md as the mdBook landing page"
```

---

### Task 3: Move the demo walkthrough into the book as `quickstart.md`

**Files:**
- Move: `demo/instructions.md` → `docs/src/quickstart.md`
- Modify: `docs/src/quickstart.md:1-3` (retitle)
- Delete: `demo/` directory (now empty)

- [ ] **Step 1: Move the file**

```bash
git mv demo/instructions.md docs/src/quickstart.md
rmdir demo
```

- [ ] **Step 2: Retitle the heading and intro**

The file currently starts with:

```markdown
# Demo: blink the on-board LED with Nucleus

This walks through setting up everything from scratch — toolchain, STM32
HAL sources, the `nucleus` CLI — and ends with a blinking LED (`LD2`, pin
`PA5`) on a **NUCLEO-F446RE**.
```

Replace just the heading line with:

```markdown
# Quickstart: Blink an LED
```

Leave the paragraph below it (and everything else in the file) unchanged.

- [ ] **Step 3: Verify the file moved and `demo/` is gone**

Run: `git status --porcelain && ls demo 2>&1`
Expected: one `R` (renamed) entry for `quickstart.md`, and `ls demo` reports
`No such file or directory`.

- [ ] **Step 4: Commit**

```bash
git add -A docs/src/quickstart.md demo
git commit -m "Move demo walkthrough into the book as the Quickstart chapter"
```

---

### Task 4: Write the new "Enabling ITM Trace" chapter

**Files:**
- Create: `docs/src/itm-trace.md`

This is the firmware-integration guide required by the Phase 8 exit criteria.
It is new content — there is currently no doc describing how to turn on
ITM/SWO from firmware. The register names and behavior below are drawn from
the ARM CoreSight CMSIS headers (`CoreDebug`, `TPI`, `ITM` structs in
`core_cm4.h`, which `stm32f4xx_hal.h` pulls in) and cross-checked against
`crates/nucleus-trace/src/source.rs` (`openocd_enable`, which sends
`tpiu config internal :<port> uart off <cpu_hz> <swo_hz>` + `itm ports on`)
and `crates/nucleus-trace/src/translate.rs` (port 0 = newline-delimited UTF-8
log lines; ports 1-7 = typed values whose `nucleus-itm` packet size is the
*access width* of the firmware write).

- [ ] **Step 1: Create `docs/src/itm-trace.md`**

```markdown
# Enabling ITM Trace

`nucleus trace` decodes ARM CoreSight ITM/SWO packets and streams them to the
trace dashboard. Getting data flowing end-to-end needs two pieces wired
together: a few lines of C in your firmware that configure the ITM/TPIU
peripherals and write to stimulus ports, and an OpenOCD session that captures
the resulting SWO byte stream and forwards it to `nucleus trace`.

```text
Firmware (ITM) --SWO--> ST-Link --USB--> OpenOCD --TCP--> nucleus trace --WebSocket--> dashboard
```

## Firmware side: the CoreSight register setup

CMSIS headers (pulled in by `stm32f4xx_hal.h`) define `CoreDebug`, `TPI`, and
`ITM` as memory-mapped structs. Enabling SWO output is six register writes:

```c
#include "stm32f4xx_hal.h"

/* Configure SWO output. `core_hz` is [device].clock_hz; `swo_hz` must match
 * [trace].swo_freq in stm32.toml (and the value passed to
 * `nucleus trace --openocd`). */
void itm_init(uint32_t core_hz, uint32_t swo_hz)
{
    CoreDebug->DEMCR |= CoreDebug_DEMCR_TRCENA_Msk; /* 1. enable tracing */

    TPI->SPPR = 2;                       /* 2. SWO protocol = NRZ/UART */
    TPI->ACPR = (core_hz / swo_hz) - 1;  /* 3. SWO baud rate divisor */

    ITM->LAR = 0xC5ACCE55;               /* 4. unlock the ITM registers */
    ITM->TCR |= ITM_TCR_ITMENA_Msk;      /* 5. enable the ITM */
    ITM->TER |= 1UL;                     /* 6. enable stimulus port 0 (log text) */
}
```

Call `itm_init(...)` once, after `SystemClock_Config()`, alongside the
generated `Nucleus_Init()`.

### Logging text (port 0)

`nucleus-trace` reassembles single-byte writes to stimulus port 0 into UTF-8
log lines, splitting on `\n`. Write one byte at a time, blocking while the
port's FIFO is full:

```c
void itm_log(const char *s)
{
    while (*s) {
        while (ITM->PORT[0].u32 == 0) { } /* wait for FIFO space */
        ITM->PORT[0].u8 = (uint8_t)*s;
        s++;
    }
}
```

A trailing `\n` flushes the line to the dashboard's log panel:

```c
itm_log("system init complete\n");
```

### Tracing typed variables (ports 1-7)

Each `[[trace.variables]]` entry in `stm32.toml` names a port and a type
(`f32`, `u16`, `u32`, or `i32`). The *access width* of the write to
`ITM->PORT[n]` determines the packet size `nucleus-itm` reports, so match the
width to the type: 4 bytes for `f32`/`u32`/`i32`, 2 bytes for `u16`.

```c
static inline void itm_write32(uint8_t port, uint32_t value)
{
    if (!(ITM->TCR & ITM_TCR_ITMENA_Msk) || !(ITM->TER & (1UL << port)))
        return;
    while (ITM->PORT[port].u32 == 0) { }
    ITM->PORT[port].u32 = value;
}

static inline void itm_write16(uint8_t port, uint16_t value)
{
    if (!(ITM->TCR & ITM_TCR_ITMENA_Msk) || !(ITM->TER & (1UL << port)))
        return;
    while (ITM->PORT[port].u16 == 0) { }
    ITM->PORT[port].u16 = value;
}
```

To trace an `f32` on port 1, write its bits as `u32`:

```c
float temperature = read_temperature();
uint32_t bits;
memcpy(&bits, &temperature, sizeof(bits));
itm_write32(1, bits);
```

Remember to enable each port you use — add it to `itm_init()`:

```c
ITM->TER |= (1UL << 1); /* enable port 1 for the temperature trace */
```

## OpenOCD side

OpenOCD captures SWO from the ST-Link and forwards it to a TCP port via
`tpiu config internal`. `nucleus trace --openocd <telnet_addr>` sends this
sequence for you over OpenOCD's telnet console (default port 4444):

```
tpiu config internal :<trace_port> uart off <core_hz> <swo_hz>
itm ports on
```

`<trace_port>` is the TCP port `nucleus trace --trace-tcp` connects to, and
`<core_hz>`/`<swo_hz>` must match the values passed to `itm_init()` in
firmware. If you'd rather configure OpenOCD by hand (e.g. to debug a version
mismatch), connect with `telnet localhost 4444` and run the same two commands.

## Putting it together

```toml
# stm32.toml
[trace]
enabled  = true
swo_freq = 2_000_000

[[trace.variables]]
name = "temperature"
port = 1
type = "f32"
```

```sh
nucleus trace --trace-tcp 127.0.0.1:3344 --openocd 127.0.0.1:4444 \
              --config stm32.toml
```

See [CLI Usage](cli.md#nucleus-trace) for the full flag reference, and open
the dashboard via the VS Code command **Nucleus: Open Trace Dashboard** (or
`extension/dist/index.html` standalone) to see the log lines and `temperature`
chart update live.
```

Note: as in Task 2, the fenced code blocks above (` ```text `, ` ```c `,
` ```toml `, ` ```sh `, and the plain ` ``` ` block for the OpenOCD commands)
are nested inside this step's outer fence — write them as literal triple-
backtick fences in the actual file.

- [ ] **Step 2: Sanity-check the C against the CMSIS struct layout used elsewhere in the workspace**

Run:
```bash
grep -rn "ITM_TCR_ITMENA_Msk\|CoreDebug_DEMCR_TRCENA_Msk\|TPI->SPPR\|ITM->LAR" cmsis-device-f4-2.6.11/ 2>/dev/null | head -5
```
Expected: at least one match for `ITM_TCR_ITMENA_Msk` and
`CoreDebug_DEMCR_TRCENA_Msk` somewhere under `cmsis-device-f4-2.6.11/` (these
are standard CMSIS-Core macro names, defined in `core_cm4.h`, vendored as part
of the CMSIS device package). If no matches are found, search for
`ITM_TCR_ITMENA` (without `_Msk`) and `TRCENA` — CMSIS versions occasionally
drop the `_Msk` suffix; adjust the macro names in `itm-trace.md` to whatever
this vendored CMSIS version actually defines, so the snippet compiles against
`STM32CUBE_PATH`.

- [ ] **Step 3: Commit**

```bash
git add docs/src/itm-trace.md
git commit -m "Add the Enabling ITM Trace chapter"
```

---

### Task 5: Write `SUMMARY.md` and verify the book builds

**Files:**
- Create: `docs/src/SUMMARY.md`
- Modify: `.gitignore` (ignore `docs/book/`)

- [ ] **Step 1: Create `docs/src/SUMMARY.md`**

```markdown
# Summary

[Introduction](introduction.md)

- [Installation](installation.md)
- [Quickstart: Blink an LED](quickstart.md)
- [CLI Usage](cli.md)
- [Enabling ITM Trace](itm-trace.md)
- [CI Integration](ci.md)
```

- [ ] **Step 2: Add `docs/book/` to `.gitignore`**

Current `.gitignore`:
```
# Rust build output
/target

# Local task notes
tasks.txt
```

Append:
```

# mdBook build output
/docs/book
```

- [ ] **Step 3: Install mdBook (if not already installed)**

Run: `cargo install mdbook --locked`
Expected: completes successfully (this compiles mdbook from crates.io; takes
a few minutes on first run). If `mdbook --version` already prints a version,
skip this step.

- [ ] **Step 4: Build the book**

Run: `mdbook build docs`
Expected: output ending in something like
`2026-06-13 ... [INFO] (mdbook::book): Running the html backend` with no
`[ERROR]` lines. `docs/book/index.html` and `docs/book/itm-trace.html` (etc.)
now exist.

- [ ] **Step 5: Spot-check the rendered output**

Run: `grep -o '<h1[^>]*>[^<]*</h1>' docs/book/itm-trace.html docs/book/introduction.html docs/book/quickstart.html`
Expected: three lines showing `Enabling ITM Trace`, `Introduction`, and
`Quickstart: Blink an LED` (mdBook may render the heading text with extra
whitespace/anchors — the key check is that each file built and contains its
expected `<h1>`).

- [ ] **Step 6: Verify `docs/book/` is ignored by git**

Run: `git status --porcelain docs/`
Expected: only `docs/src/SUMMARY.md`, `docs/book.toml` (if not yet committed),
and `.gitignore` show as new/modified — `docs/book/` does not appear.

- [ ] **Step 7: Commit**

```bash
git add docs/src/SUMMARY.md .gitignore
git commit -m "Add SUMMARY.md and ignore the mdBook build output"
```

---

### Task 6: Add the GitHub Pages deploy workflow

**Files:**
- Create: `.github/workflows/docs.yml`

- [ ] **Step 1: Create `.github/workflows/docs.yml`**

```yaml
name: docs

on:
  push:
    branches: [main]
    paths:
      - "docs/**"
      - ".github/workflows/docs.yml"
  workflow_dispatch:

concurrency:
  group: pages
  cancel-in-progress: false

permissions:
  contents: read
  pages: write
  id-token: write

env:
  CARGO_TERM_COLOR: always
  MDBOOK_VERSION: 0.4.40

jobs:
  build:
    name: build book
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2
        with:
          key: mdbook-${{ env.MDBOOK_VERSION }}

      - name: Install mdbook
        run: cargo install mdbook --version ${{ env.MDBOOK_VERSION }} --locked

      - name: Build book
        run: mdbook build docs

      - name: Setup Pages
        uses: actions/configure-pages@v5

      - name: Upload artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: docs/book

  deploy:
    name: deploy to GitHub Pages
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 2: Validate the YAML**

Run:
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/docs.yml'))" && echo OK
```
Expected: `OK`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/docs.yml
git commit -m "Add GitHub Pages deploy workflow for the mdBook docs site"
```

Note: after this is merged, a one-time manual step is required (not
achievable via a commit): in the repo's **Settings → Pages**, set **Source**
to **GitHub Actions**. Until that's done, the workflow will build successfully
but the `deploy` job will fail with a "Pages site not configured" error — flag
this to the user after the implementation lands.

---

### Task 7: Add GitHub issue templates

**Files:**
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`
- Create: `.github/ISSUE_TEMPLATE/feature_request.yml`
- Create: `.github/ISSUE_TEMPLATE/config.yml`

- [ ] **Step 1: Create `.github/ISSUE_TEMPLATE/bug_report.yml`**

```yaml
name: Bug report
description: Report a problem with the nucleus CLI, LSP, trace, or VS Code extension.
labels: ["bug"]
body:
  - type: textarea
    id: description
    attributes:
      label: What happened?
      description: A clear description of the bug.
    validations:
      required: true
  - type: textarea
    id: repro
    attributes:
      label: Reproduction
      description: The stm32.toml (or relevant snippet) and the exact command you ran.
      render: toml
    validations:
      required: true
  - type: textarea
    id: expected
    attributes:
      label: Expected vs. actual behavior
    validations:
      required: true
  - type: input
    id: version
    attributes:
      label: nucleus --version output
    validations:
      required: true
  - type: dropdown
    id: board
    attributes:
      label: Board / family
      options:
        - NUCLEO-F446RE (STM32F446RE)
        - NUCLEO-F411RE (STM32F411RE)
        - Other / not hardware-specific
    validations:
      required: true
  - type: dropdown
    id: os
    attributes:
      label: Operating system
      options:
        - Linux
        - macOS
        - Windows
    validations:
      required: true
  - type: textarea
    id: logs
    attributes:
      label: Relevant log output
      render: shell
```

- [ ] **Step 2: Create `.github/ISSUE_TEMPLATE/feature_request.yml`**

```yaml
name: Feature request
description: Suggest an idea for Nucleus.
labels: ["enhancement"]
body:
  - type: textarea
    id: problem
    attributes:
      label: What problem does this solve?
    validations:
      required: true
  - type: textarea
    id: solution
    attributes:
      label: Proposed solution
    validations:
      required: true
  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives considered
  - type: dropdown
    id: component
    attributes:
      label: Which component does this touch?
      multiple: true
      options:
        - nucleus-cli
        - nucleus-compiler
        - nucleus-db
        - nucleus-lsp
        - nucleus-itm
        - nucleus-trace
        - VS Code extension
        - Documentation
    validations:
      required: true
```

- [ ] **Step 3: Create `.github/ISSUE_TEMPLATE/config.yml`**

```yaml
blank_issues_enabled: false
contact_links:
  - name: Questions & discussion
    url: https://github.com/harshverma27/nucleus/blob/main/CONTRIBUTING.md
    about: Check CONTRIBUTING.md for the build/test workflow before opening an issue.
```

- [ ] **Step 4: Validate all three YAML files**

Run:
```bash
for f in .github/ISSUE_TEMPLATE/*.yml; do
  python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" "$f" && echo "$f OK"
done
```
Expected:
```
.github/ISSUE_TEMPLATE/bug_report.yml OK
.github/ISSUE_TEMPLATE/config.yml OK
.github/ISSUE_TEMPLATE/feature_request.yml OK
```

- [ ] **Step 5: Commit**

```bash
git add .github/ISSUE_TEMPLATE/
git commit -m "Add bug report and feature request issue templates"
```

---

### Task 8: Update root README, CHANGELOG, and CLAUDE.md

**Files:**
- Modify: `README.md:11-13` (add docs link)
- Modify: `README.md:474-484` (Phase 8 status line)
- Modify: `CHANGELOG.md:11-20` (Unreleased section)
- Modify: `CLAUDE.md` (Phase 8 status paragraph)

- [ ] **Step 1: Add a documentation link near the top of `README.md`**

Current `README.md:5-13`:
```markdown
**Not an IDE replacement. A developer platform.**

Nucleus solves the two real lock-ins keeping embedded developers on STM32CubeIDE:
1. Graphical pin/peripheral configuration that produces opaque, un-diffable XML
2. Integrated debug/trace tooling with no open-source equivalent

Nucleus replaces both with a CLI-first, version-controllable, CI-friendly workflow that lives inside VS Code — or any editor.

---
```

Insert a line after the last sentence and before the `---`:

```markdown
**Not an IDE replacement. A developer platform.**

Nucleus solves the two real lock-ins keeping embedded developers on STM32CubeIDE:
1. Graphical pin/peripheral configuration that produces opaque, un-diffable XML
2. Integrated debug/trace tooling with no open-source equivalent

Nucleus replaces both with a CLI-first, version-controllable, CI-friendly workflow that lives inside VS Code — or any editor.

📖 **[Read the docs](https://harshverma27.github.io/nucleus/)**

---
```

- [ ] **Step 2: Add a Phase 8 status line in `README.md`**

Current `README.md:474-480`:
```markdown
### Phase 8 — Docs, Generality Proof + Community Launch

**Goal:** Production-quality, documented, and proven to generalize beyond one chip.

Scope: documentation, a second MCU family (STM32F411RE), and public launch.

**Exit criteria:**
```

Insert a status line after the heading, matching the format used by Phases
1-7:

```markdown
### Phase 8 — Docs, Generality Proof + Community Launch

> **Status: 🟡 In progress.** The mdBook docs site (published on GitHub
> Pages, including the ITM/OpenOCD firmware integration guide) is live,
> STM32F411RE (NUCLEO-F411RE) is supported end-to-end, CI gates `check` +
> `build` + `test` on every PR, and CONTRIBUTING + issue templates are in
> place. The demo video and public launch posts remain.

**Goal:** Production-quality, documented, and proven to generalize beyond one chip.

Scope: documentation, a second MCU family (STM32F411RE), and public launch.

**Exit criteria:**
```

- [ ] **Step 3: Add a CHANGELOG entry**

Current `CHANGELOG.md:11-20` (the `## [Unreleased]` section's first bullet
list item):

```markdown
## [Unreleased]

### Added
- **STM32F411RE support.** The NUCLEO-F411RE is a fully supported second board:
  `family = "STM32F411RE"` validates against a dedicated constraint database
  (generated from ST open pin data), `nucleus init --board NUCLEO-F411RE`
  scaffolds an F411-specific project, and `nucleus build` generates HAL code for
  it. A new `PeripheralUnavailable` conflict flags peripherals absent on the
  selected family, and the LSP resolves diagnostics/hover against the document's
  family. This fulfills the Phase 8 generality-proof criterion.
```

Add a new bullet immediately after that one (before the "Phase 7" bullet):

```markdown
- **mdBook docs site + issue templates.** The `docs/` directory is now an
  mdBook source tree (Introduction, Installation, Quickstart, CLI Usage,
  Enabling ITM Trace, CI Integration), built and published to GitHub Pages on
  every push to `main` that touches `docs/`. The new "Enabling ITM Trace"
  chapter covers the firmware-side CoreSight register setup and the matching
  OpenOCD `tpiu`/`itm` commands. `.github/ISSUE_TEMPLATE/` adds structured bug
  report and feature request forms.
```

- [ ] **Step 4: Update the Phase 8 status note in `CLAUDE.md`**

In `CLAUDE.md`, the "Project status" section currently ends its Phase 8
paragraph with:

```
Remaining Phase 8 work: mdBook docs on GitHub Pages, CONTRIBUTING/issue templates, demo + launch.
```

Replace that sentence with:

```
Remaining Phase 8 work: demo video + public launch (both maintainer/recording steps, not code).
```

- [ ] **Step 5: Verify the README and CHANGELOG render sensibly**

Run:
```bash
grep -n "Read the docs\|Status: 🟡 In progress" README.md
grep -n "mdBook docs site + issue templates" CHANGELOG.md
grep -n "Remaining Phase 8 work" CLAUDE.md
```
Expected: each grep returns exactly one matching line.

- [ ] **Step 6: Commit**

```bash
git add README.md CHANGELOG.md CLAUDE.md
git commit -m "Update README/CHANGELOG/CLAUDE.md for Phase 8 docs completion"
```

---

### Task 9: Full local gate

**Files:** none (verification only)

- [ ] **Step 1: Run the full local gate**

Run: `make check`
Expected: passes (this task touched no Rust source, so `fmt-check`/`lint`/
`test` should be unaffected — this just confirms nothing was accidentally
broken, e.g. by the `.gitignore` change).

- [ ] **Step 2: Rebuild the book one more time from a clean state**

Run:
```bash
rm -rf docs/book
mdbook build docs
ls docs/book/index.html docs/book/itm-trace.html docs/book/quickstart.html docs/book/cli.html docs/book/installation.html docs/book/ci.html
```
Expected: all six files listed exist.

- [ ] **Step 3: Final review of changed files**

Run: `git log --oneline -8 && git status --porcelain`
Expected: 8 new commits from Tasks 1-8 (one per task), working tree clean
(`docs/book/` ignored, not shown).
