# Design: mdBook Docs Site, GitHub Pages, Issue Templates

**Date:** 2026-06-13
**Status:** Approved, ready for implementation planning

## Goal

Close out the remaining Phase 8 exit criteria (per README "Phase 8 — Docs,
Generality Proof + Community Launch"):

- mdBook docs site published on GitHub Pages, including a firmware
  integration guide for enabling ITM trace (OpenOCD config + the CoreSight
  register setup in C).
- Issue templates in place (CONTRIBUTING.md already exists from Phase 7).

The STM32F411RE generality criterion is already done (separate spec/PR). The
demo video recording and public launch posts are explicitly out of scope —
the user records the demo themselves; this work only needs to leave a
walkthrough doc the recording can follow.

## 1. Book structure (`docs/`)

Restructure `docs/` into an mdBook source tree, since `docs/README.md`
already designates these pages as "the canonical reference until" the mdBook
lands:

```
docs/
  book.toml
  src/
    SUMMARY.md
    introduction.md
    installation.md
    quickstart.md
    cli.md
    itm-trace.md
    ci.md
```

- `book.toml`: `title = "Nucleus"`, `src = "src"`, repo link set to
  `https://github.com/harshverma27/nucleus` for "edit this page" links.
- `introduction.md`: derived from current `docs/README.md`. Drop the
  "lands in Phase 8" / "until then" hedges since Phase 8 is now done; keep
  the "At a glance" `stm32.toml` example and command summary. Add a short
  nav blurb pointing to the other chapters (mdBook's sidebar makes the
  manual links in the old `docs/README.md` redundant, but a one-paragraph
  overview stays).
- `installation.md`, `cli.md`, `ci.md`: moved verbatim (`git mv`), content
  unchanged — they're already accurate.
- `quickstart.md`: moved from `demo/instructions.md` (`git mv`), with:
  - Title/intro reworded slightly for book context (it's currently framed
    as "Demo: blink the on-board LED" — becomes "Quickstart: Blink an LED",
    keeping all the install/build/flash/troubleshooting content as-is).
  - `demo/` directory removed entirely (nothing else references it).
- `SUMMARY.md`:
  ```
  # Summary

  - [Introduction](introduction.md)
  - [Installation](installation.md)
  - [Quickstart: Blink an LED](quickstart.md)
  - [CLI Usage](cli.md)
  - [Enabling ITM Trace](itm-trace.md)
  - [CI Integration](ci.md)
  ```

## 2. New chapter: `itm-trace.md` ("Enabling ITM Trace")

This is new content (nothing in the repo currently documents ITM enablement
from the firmware side). Written from the ARM CoreSight spec
(DDI0403E) and cross-checked against what `nucleus-trace` already implements
in `crates/nucleus-trace/src/source.rs` (`openocd_enable`) and
`crates/nucleus-trace/src/translate.rs` (port 0 = log lines, ports 1-7 =
typed `[[trace.variables]]`).

Structure:

1. **Overview** — one paragraph: SWO carries ITM packets out over the
   ST-Link SWD connector; OpenOCD captures them on a TCP port;
   `nucleus trace` decodes and serves them to the dashboard. Diagram already
   exists in `crates/nucleus-trace/README.md` — reuse/adapt it.

2. **Firmware side: the CoreSight register setup.** A minimal `itm_init()`
   (or inline snippet) using core CMSIS registers (`CoreDebug`, `ITM`,
   `TPI`), covering:
   - `CoreDebug->DEMCR |= CoreDebug_DEMCR_TRCENA_Msk` — enable tracing.
   - `TPI->SPPR = 2` — SWO protocol = NRZ/UART (matches OpenOCD's
     `tpiu config internal ... uart off ...`).
   - `TPI->ACPR = (core_clk / swo_freq) - 1` — SWO baud divisor, where
     `swo_freq` matches `[trace].swo_freq` in `stm32.toml`.
   - `ITM->LAR = 0xC5ACCE55` then `ITM->TCR |= ITM_TCR_ITMENA_Msk` — unlock
     and enable ITM.
   - `ITM->TER |= (1UL << port)` — enable stimulus port(s) (port 0 for log
     text, plus any ports used by `[[trace.variables]]`).
   - A tiny blocking write helper (`ITM_SendChar`-style, polling
     `ITM->PORT[n].u32` busy bit) for port 0 string logging and for writing
     typed values (`f32`/`u16`/`u32`/`i32`) to variable ports — matching
     `VarType` in `translate.rs`.

3. **OpenOCD side.** The exact `tpiu config internal :<port> uart off
   <cpu_hz> <swo_hz>` + `itm ports on` telnet sequence from
   `source::openocd_enable`, with a note that this is the same sequence
   `nucleus trace --openocd <telnet_addr>` sends automatically — manual
   telnet is only needed for debugging OpenOCD-version mismatches.

4. **Wiring it to `nucleus trace`.** Cross-reference the `cli.md` chapter's
   `nucleus trace` section; show a complete `[trace]` block in `stm32.toml`
   (`enabled`, `swo_freq`, one `[[trace.variables]]` entry) paired with the
   firmware snippet from step 2.

No source crates change — this is documentation only.

## 3. GitHub Pages deploy workflow

New `.github/workflows/docs.yml`:

- Trigger: `push` to `main` with `paths: ["docs/**"]`, plus
  `workflow_dispatch` for manual re-runs.
- Permissions: `contents: read`, `pages: write`, `id-token: write`.
- `concurrency`: group `pages`, no cancel-in-progress (avoid interrupting a
  live deploy).
- Steps: checkout, install `mdbook` (`cargo install mdbook --locked`, cached
  via `Swatinem/rust-cache` keyed on the mdbook version to avoid rebuilding
  every run), `mdbook build docs` (outputs to `docs/book/`), then
  `actions/configure-pages`, `actions/upload-pages-artifact` (path
  `docs/book`), `actions/deploy-pages`.
- `docs/book/` (the build output) gets a `.gitignore` entry — it's a build
  artifact, not checked in.

**Manual one-time step (not done by this change):** repo Settings → Pages →
Source = "GitHub Actions". This can't be done via a commit; call it out in
the PR description / a follow-up note to the user.

## 4. Issue templates

`.github/ISSUE_TEMPLATE/`:

- `bug_report.yml` — GitHub issue form with fields: description, repro
  steps (`stm32.toml` snippet + command run), expected vs. actual,
  `nucleus --version` output, OS, board/family (`NUCLEO-F446RE` /
  `NUCLEO-F411RE`), relevant log output.
- `feature_request.yml` — fields: problem/motivation, proposed solution,
  alternatives considered, which Nucleus component it touches
  (`nucleus-cli` / `-compiler` / `-db` / `-lsp` / `-itm` / `-trace` /
  extension).
- `config.yml` — `blank_issues_enabled: false`, with a contact link back to
  `CONTRIBUTING.md` for questions/discussion.

## 5. Cross-doc updates

- Root `README.md`: add a "Documentation" link near the top pointing at the
  published Pages URL (`https://harshverma27.github.io/nucleus/`), and
  update the Phase 8 status line once this lands.
- `CHANGELOG.md`: add a new bullet under "Unreleased" / "Added" noting the
  published mdBook docs site (with the ITM trace integration guide) and the
  new issue templates, alongside the existing F411RE bullet.
- `CLAUDE.md`: Phase 8 status note updated to reflect only "demo + launch"
  (a human/recording step) remaining.

## Out of scope

- Demo video recording and public launch posts (user does these).
- Any change to crate source code, CI test/build jobs, or release workflow.
- JetBrains/Neovim extensions (explicitly post-Marketplace per README scope
  rules, unaffected by this work).
