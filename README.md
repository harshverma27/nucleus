# Nucleus

> A modern STM32 developer platform for declarative hardware configuration and real-time trace debugging.

**Not an IDE replacement. A developer platform.**

Nucleus solves the two real lock-ins keeping embedded developers on STM32CubeIDE:
1. Graphical pin/peripheral configuration that produces opaque, un-diffable XML
2. Integrated debug/trace tooling with no open-source equivalent

Nucleus replaces both with a CLI-first, version-controllable, CI-friendly workflow that lives inside VS Code — or any editor.

---

## Table of Contents

- [The Problem](#the-problem)
- [What Nucleus Is](#what-nucleus-is)
- [Architecture](#architecture)
- [Components](#components)
  - [nucleus CLI](#1-nucleus-cli-rust)
  - [Pinmux Compiler](#2-pinmux-compiler-rust)
  - [ITM Trace Daemon](#3-itm-trace-daemon-rust)
  - [VS Code Extension](#4-vs-code-extension-typescript--react)
- [The stm32.toml Format](#the-stm32toml-format)
- [Tech Stack](#tech-stack)
- [Phased Roadmap](#phased-roadmap)
  - [Phase 1 — Constraint Database Foundation](#phase-1--constraint-database-foundation)
  - [Phase 2 — Config Parser + Constraint Solver](#phase-2--config-parser--constraint-solver)
  - [Phase 3 — HAL Code Generation + Build](#phase-3--hal-code-generation--build)
  - [Phase 4 — LSP Server + Editor UX](#phase-4--lsp-server--editor-ux)
  - [Phase 5 — ITM Decoder + Trace Backend](#phase-5--itm-decoder--trace-backend)
  - [Phase 6 — Trace Dashboard](#phase-6--trace-dashboard)
  - [Phase 7 — Distribution + Release Automation](#phase-7--distribution--release-automation)
  - [Phase 8 — Docs, Generality Proof + Community Launch](#phase-8--docs-generality-proof--community-launch)
- [Known Hard Problems](#known-hard-problems)
- [Scope Discipline Rules](#scope-discipline-rules)
- [Naming](#naming)
- [Resume Line](#resume-line)

---

## The Problem

Every STM32 developer knows this workflow:

1. Open STM32CubeIDE (2 GB Eclipse fork, 40 second startup)
2. Click through CubeMX pin assignment GUI
3. Get a generated project full of `MX_Init` boilerplate and opaque XML
4. Try to diff it in git — impossible
5. Try to run it in CI — impossible without a full GUI install
6. Debug with `printf` over UART because ITM tracing requires either a $600 Segger license or raw OpenOCD byte dumps nobody can read

Nucleus fixes both of these with a 4 MB CLI and a VS Code extension.

---

## What Nucleus Is

Three products inside one ecosystem:

| Product | What it does |
|---|---|
| Hardware-aware build system | `stm32.toml` → validated HAL init code → CMake project |
| Language tooling platform | LSP server with live conflict detection as you edit config |
| Embedded observability stack | ITM packet decoder → WebSocket → real-time React dashboard |

---

## Installation

```sh
# From crates.io (once published — see note below)
cargo install nucleus-cli

# From source, today
cargo install --git https://github.com/harshverma27/nucleus nucleus-cli --locked
```

Tagged releases also attach prebuilt binaries for Linux/macOS/Windows (x86_64 +
arm64). The VS Code extension is a thin client installed from the Marketplace or
a release `.vsix`. Full details — including the from-source path used before the
first crates.io/Marketplace publish — are in **[`docs/installation.md`](docs/installation.md)**;
CLI reference in [`docs/cli.md`](docs/cli.md); CI gating in [`docs/ci.md`](docs/ci.md).

---

## Architecture

```
nucleus CLI  (Rust — primary product, owns all logic)
    │
    ├── pinmux compiler     parse stm32.toml → solve constraints → emit C
    │       └── LSP server  expose diagnostics + hover over language server protocol
    │
    ├── ITM trace daemon    read SWO from OpenOCD → decode CoreSight packets → WebSocket
    │
    └── build orchestrator  invoke CMake + arm-none-eabi-gcc + st-flash

VS Code Extension  (TypeScript — thin UI layer only)
    ├── LSP client          talks to nucleus lsp, shows squiggles in editor
    └── Webview panel       embeds React ITM dashboard
```

**The CLI owns everything. The extension is a display layer.**

This means:
- CI works without VS Code installed
- Future JetBrains / Neovim support requires only a new thin client, not a rewrite
- The hard logic lives in one place, tested independently

---

## Components

### 1. nucleus CLI (Rust)

The backbone everything else builds on. Acts as `cargo` does for Rust projects.

**Commands:**

| Command | What it does |
|---|---|
| `nucleus init` | Scaffold a new STM32 project: `stm32.toml`, `CMakeLists.txt`, `src/main.c`, `.github/workflows/ci.yml`. Use `--board` to choose the target (`NUCLEO-F446RE` default, or `NUCLEO-F411RE`) |
| `nucleus check` | Validate `stm32.toml` against the constraint database, print conflicts |
| `nucleus build` | Run CMake + arm-none-eabi-gcc, emit firmware `.elf` and `.bin` |
| `nucleus flash` | Invoke `st-flash` or OpenOCD to program the Nucleo board |
| `nucleus trace` | Start the ITM daemon + open the dashboard in VS Code or browser |
| `nucleus lsp` | Start the language server (called by the VS Code extension, not by humans) |

All commands are composable and scriptable. `nucleus check` exits non-zero on conflicts — CI can gate on it.

---

### 2. Pinmux Compiler (Rust)

**Input:** `stm32.toml`
**Output:** `nucleus_init.c` + `nucleus_init.h` containing clean `MX_`-style HAL initialization

**What it actually is:** A hardware constraint solver. Not a code formatter.

#### Constraint database

Built from STM32 CMSIS Pack alternate function tables (publicly available from ST). The database maps every pin on every supported MCU to:
- Which alternate functions are available (AF0–AF15)
- Which peripheral each AF connects to
- Which clock domain feeds each peripheral
- Which DMA streams/channels each peripheral can use (future scope)

For Phase 1, the database covers exactly one MCU: **STM32F446RE** (the chip on the NUCLEO-F446RE). Expanding to other families is additive later.

#### Conflict detection (what the compiler catches)

- **Pin collision:** Two peripherals configured on the same physical pin
- **AF mismatch:** A pin assigned to a peripheral it doesn't physically connect to on that MCU
- **Clock domain disabled:** A peripheral configured but its bus clock (`APB1`/`APB2`) not enabled
- **Missing pins:** A peripheral declared without all required pins (e.g. SPI without MOSI)

#### What the compiler deliberately skips (early phases)

- DMA channel collision detection — hard, device-specific, low perceived value early on
- Full clock tree validation — complex, not scheduled (only basic clock-domain checks ship, in Phase 2)
- Middleware integration — too much maintenance burden

#### HAL codegen strategy (important architectural decision)

Do **not** generate giant monolithic HAL boilerplate. CubeMX already does that and it becomes a maintenance nightmare as ST updates HAL APIs.

Instead, generate:
- A `nucleus_config.h` containing typed config structs for each peripheral
- A `nucleus_init.c` containing a single `Nucleus_Init()` function that calls standard ST HAL functions with the resolved parameters

The generated code calls `HAL_UART_Init()`, `HAL_SPI_Init()` etc. with correct parameters. It does not reimplement the HAL. This means ST HAL updates don't break Nucleus — the compiler just calls the updated API with the right arguments.

---

### 3. ITM Trace Daemon (Rust)

**This is the most technically elite component. It requires reading the ARM CoreSight architecture specification and implementing binary protocol decoding from scratch.**

#### What ITM is

STM32 microcontrollers have an on-chip Instrumentation Trace Macrocell (ITM) — part of ARM's CoreSight debug architecture. It lets firmware stream structured data to a debugger over the SWO (Single Wire Output) pin, which on Nucleo boards is already wired to the ST-Link on-board debugger. No extra hardware required.

The problem: the only way to read ITM data today is raw bytes from OpenOCD, or Segger Ozone ($600 commercial license). No open source tool decodes it cleanly into a dashboard.

#### How the daemon works

```
Nucleo board (SWO pin)
    → ST-Link on-board debugger (USB)
        → OpenOCD (running locally, telnet interface on port 4444)
            → nucleus ITM daemon (connects to OpenOCD telnet, reads raw SWO byte stream)
                → CoreSight packet decoder (ARM spec implementation in Rust)
                    → structured JSON events
                        → WebSocket server (port 7878)
                            → React dashboard (in VS Code Webview or browser)
```

#### Packet types decoded

- **SWIT (Software Instrumentation Trace) packets — Port 0:** Decoded as UTF-8 log messages. This replaces `printf` over UART — faster, zero-wasted CPU cycles on waiting for UART TX.
- **SWIT packets — Ports 1–7:** Decoded as typed variable values (f32, u16, i32 etc.) for live variable tracing. Firmware calls `ITM_SendChar()` with a port number encoding the variable identity.
- **DWT (Data Watchpoint and Trace) packets:** CPU cycle counter data, used for load estimation. Phase 6 only.

#### What firmware needs (minimal)

```c
// In firmware — replaces printf
void nucleus_log(const char *msg) {
    while (*msg) {
        ITM_SendChar((uint32_t)*msg++);
    }
}

// Trace a variable on port 1
void nucleus_trace_f32(uint8_t port, float value) {
    union { float f; uint32_t u; } v = { .f = value };
    ITM->PORT[port].u32 = v.u;
}
```

No library dependency. Four lines of C. Works on any STM32 with ITM enabled in OpenOCD config.

---

### 4. VS Code Extension (TypeScript + React)

**Thin UI layer. Contains zero logic.**

Two features:

#### LSP client
Connects to `nucleus lsp` (started automatically when a project with `stm32.toml` is opened). Provides:
- Red squiggles on pin conflicts as you type
- Yellow warnings on missing optional pins
- Hover docs showing what AF number a pin uses and what the datasheet says about it
- Autocomplete for pin names (`PA`, `PB`, `PC`... with valid options for the selected MCU)

#### ITM dashboard Webview
A React panel embedded in VS Code. Connects to the daemon's WebSocket. Three panels:

**Log stream panel** — decoded port 0 output. Timestamped, filterable, searchable. Replaces the serial monitor for debug logging. Each message shows the source port and absolute timestamp from the ITM timestamp packets.

**Variable timeline panel** — each traced variable (ports 1–7) plotted as a live time-series chart on a Canvas element. X-axis is wall clock time, Y-axis auto-scales to the value range. Up to 7 simultaneous variables. New data points stream in at whatever rate the firmware emits them — typically sub-millisecond latency.

**CPU load panel** — derived from DWT cycle counter packets, showing approximate CPU utilization as a rolling strip chart. Phase 6 only.

---

## The stm32.toml Format

```toml
[device]
family  = "STM32F446RE"     # MCU part number — determines constraint database
board   = "NUCLEO-F446RE"   # optional, enables board-specific defaults
clock_hz = 180_000_000      # SYSCLK in Hz

[build]
toolchain = "arm-none-eabi-gcc"
optimization = "Os"         # passed to CMake as -O flag

# ── Peripherals ────────────────────────────────────────────────────────────────

[peripherals.usart2]        # maps to HAL_UART_Init
tx   = "PA2"
rx   = "PA3"
baud = 115200

[peripherals.spi1]          # maps to HAL_SPI_Init
mosi = "PA7"
miso = "PA6"
sck  = "PA5"
nss  = "PA4"
mode = 0                    # SPI mode 0–3, default 0

[peripherals.i2c1]          # maps to HAL_I2C_Init
sda = "PB9"
scl = "PB8"
speed = "standard"          # "standard" (100kHz) | "fast" (400kHz)

[peripherals.tim2]          # maps to HAL_TIM_PWM_Init
channel1 = "PA0"
channel2 = "PA1"
frequency_hz = 1000
duty_resolution_bits = 16

# ── Trace configuration ────────────────────────────────────────────────────────

[trace]
enabled  = true
swo_freq = 2_000_000        # SWO clock in Hz — must be divisible into SYSCLK

[[trace.variables]]
name = "temperature"
port = 1                    # ITM stimulus port 1
type = "f32"

[[trace.variables]]
name = "duty_cycle"
port = 2
type = "u16"

[[trace.variables]]
name = "loop_time_us"
port = 3
type = "u32"
```

**Why this format is powerful:**
- Fully version-controllable and diffable in git
- Reviewable in PRs — a reviewer can see "pin PA5 moved from SPI1 to SPI2" as a one-line diff
- CI-runnable — `nucleus check` validates it in a GitHub Actions job with no GUI
- Human-writable — a 20-line file replaces an hour of clicking through CubeMX menus

---

## Tech Stack

| Layer | Technology | Why |
|---|---|---|
| CLI + compiler + LSP server + ITM daemon | Rust | Memory safety, speed, cargo ecosystem, elite signal to research labs |
| Constraint database source | STM32 CMSIS Packs + SVD files (public, from ST) | Official source of truth for pin/AF data |
| Language server protocol | `tower-lsp` crate | Async LSP server framework, well maintained |
| TOML parsing | `toml` crate | Obvious choice for the config format |
| ITM packet decoding | Hand-rolled Rust (ARM CoreSight spec) | No existing crate covers this correctly |
| WebSocket server | `tokio-tungstenite` | Async, integrates with tokio runtime |
| VS Code extension host | TypeScript | Required by VS Code extension API |
| ITM dashboard | React + Canvas API | Canvas for high-frequency live chart rendering |
| Build system integration | CMake + arm-none-eabi-gcc | Standard embedded toolchain |
| Debug probe interface | OpenOCD telnet API | Open source, works with ST-Link |
| CI integration | GitHub Actions | Standard, widely understood |

---

## Phased Roadmap

Scope discipline is the biggest risk on this project. Each phase ships something real and usable before the next begins, and is gated by **exit criteria** — measurable conditions that must hold before the next phase starts. Phases ship when their criteria are met, not on a calendar.

**The end product:** a polished, open-source STM32 toolchain a stranger can install in one command (`cargo install` + VS Code Marketplace), with GitHub Actions automating cross-platform releases. A published tool with real users is the goal.

**Target hardware: NUCLEO-F446RE only** through Phase 7. The NUCLEO-F411RE lands in Phase 8 to prove the design generalizes.

---

### Phase 1 — Constraint Database Foundation

> **Status: ✅ Complete.** `nucleus-db` ships the byte-deterministic F446RE pin/AF/peripheral table with lookup APIs, generated at build time from the vendored pack data.

**Goal:** Turn the vendored CMSIS F4 pack into an embedded, deterministic pin/AF/peripheral database for the STM32F446RE.

Scope: `nucleus-db`. Parse the alternate-function tables (all GPIOs PA0–PC15, all AF0–AF15, peripheral-to-pin mappings), normalize inconsistent pack data, and embed the result at compile time (`build.rs` or `xtask`). Known pack errors go in a hand-maintained patch table.

**Exit criteria:**
- `nucleus-db` exposes pin ↔ AF ↔ peripheral lookup APIs.
- The compiled database is byte-deterministic across builds (required for testable CI).
- Unit tests assert known mappings (e.g. PA7 → AF5 = SPI1_MOSI) and negative cases.
- Pack-data anomalies are recorded in the patch table for traceability.

---

### Phase 2 — Config Parser + Constraint Solver

> **Status: ✅ Complete.** `nucleus-compiler` parses `stm32.toml` and solves the four conflict classes; `nucleus check` surfaces them and exits non-zero on any error.

**Goal:** Parse `stm32.toml` and catch every Phase-1-class conflict.

Scope: `nucleus-compiler` parser + solver, surfaced via `nucleus check`. Conflict classes: pin collision, AF mismatch, missing required pins, clock domain disabled. No DMA collision detection; no full clock-tree solving (only "is the bus clock enabled?").

Basic clock-domain checking reads an optional `[clocks]` section in `stm32.toml`; each bus (`ahb1`/`apb1`/`apb2`) defaults to enabled, so omitting the section never produces a false "clock disabled" error:

```toml
[clocks]
apb1 = true
apb2 = true
```

**Exit criteria:**
- `nucleus check` reads `stm32.toml`, prints conflicts with pin names and descriptions, and exits non-zero on any error (so CI can gate on it). ✅
- All four conflict classes are detected and unit-tested. ✅
- Integration test: a `stm32.toml` with a deliberate PA5 collision produces exactly one error. ✅ (`tests/fixtures/pa5_collision.toml`, driven by `crates/nucleus-cli/tests/cli.rs`)

---

### Phase 3 — HAL Code Generation + Build

> **Status: ✅ Complete** (on-hardware validation requires the user's cross toolchain + board — see note). Codegen lives in `nucleus-compiler::codegen`; `nucleus init`/`build`/`flash` orchestrate scaffolding and the toolchain.

**Goal:** Go from validated config to flashed firmware.

Scope: codegen in `nucleus-compiler`; orchestration in `nucleus-cli` (`init`, `build`, `flash`). Generated code calls stock ST HAL `Init` functions with resolved parameters — it does **not** reimplement the HAL.

**Exit criteria:**
- `nucleus init` scaffolds a project (`stm32.toml`, `CMakeLists.txt`, `cmake/` toolchain file, `src/main.c`, CI workflow). ✅ (idempotent; never overwrites)
- Codegen emits `nucleus_config.h` (typed per-peripheral config structs) and `nucleus_init.c` (a single `Nucleus_Init()` calling `HAL_*_Init` functions, with GPIO alternate-function muxing resolved from `nucleus-db`). ✅
- `nucleus build` validates the config, regenerates the sources, and drives CMake + arm-none-eabi-gcc to produce `.elf`/`.bin`; `nucleus flash` programs the board with `st-flash`. ✅ (build refuses a conflicting config; missing toolchain yields a clear, actionable error)
- Generated init code compiles and correctly initializes peripherals on a real NUCLEO-F446RE. ⚙️ *Requires the ARM cross toolchain + an STM32CubeF4 (HAL) checkout + the physical board; the scaffolded `CMakeLists.txt` wires these via `STM32CUBE_PATH`. Codegen output structure is unit/integration-tested in CI; the physical flash is a user/maintainer step on hardware.*

---

### Phase 4 — LSP Server + Editor UX

> **Status: ✅ Complete.** `nucleus-lsp` (tower-lsp) serves diagnostics, hover, and pin completion; `nucleus lsp` starts it over stdio, and the VS Code extension is a thin client that spawns it.

**Goal:** Live config feedback inside the editor.

Scope: `nucleus-lsp` (`tower-lsp`) wrapping the compiler, plus the VS Code extension's LSP client. The extension stays a thin client — zero business logic.

**Exit criteria:**
- `nucleus lsp` serves conflict diagnostics (published on open/change), `textDocument/hover` (the pin's full AF table from `nucleus-db`), and pin-name completion for the selected MCU. ✅
- The extension activates on opening a `stm32.toml`, spawns `nucleus lsp`, and connects the client. ✅ (`extension/src/extension.ts`)
- Demo: opening a config with a pin conflict shows a red squiggle in VS Code as you type. ✅ (a PA5 collision publishes one ERROR diagnostic per colliding pin site; the analysis layer is unit-tested and the stdio server is verified end-to-end)

The conflict→squiggle mapping lives in `nucleus-lsp::analysis` as pure functions (text → diagnostics/hover/completions), so the editor behaviour is fast and deterministic to unit-test; `analysis` maps each conflict to the most relevant source span (a collision underlines every colliding pin; a missing/clock conflict underlines the table header).

---

### Phase 5 — ITM Decoder + Trace Backend

> **Status: ✅ Complete** (live-hardware capture is a maintainer step — see note). `nucleus-itm` decodes the CoreSight stream; `nucleus-trace` translates and streams it; `nucleus trace` runs the daemon.

**Goal:** Decode CoreSight ITM from the SWO pin and stream it to clients.

Scope: `nucleus-itm` (decoder) and `nucleus-trace` (OpenOCD telnet integration + WebSocket server), surfaced via `nucleus trace`. Implements the ARM CoreSight packet format from the spec.

**Exit criteria:**
- Decoder handles SWIT port 0 (UTF-8 logs) and ports 1–7 (typed values: f32, u16, u32, i32), matching port numbers to `[trace.variables]` names. ✅ (decode in `nucleus-itm`, port→name/type mapping in `nucleus-trace::translate`)
- Decoder survives framing edge cases — packets spanning read boundaries, overflow packets, resync after a dropped connection — with **zero panics under fuzzing**. ✅ (zero-dependency, length-checked decoder; a randomized test feeds thousands of arbitrary byte streams in arbitrary chunk sizes and asserts no panic + chunk-invariance; the buffer is capped for O(1) memory)
- `nucleus trace` reads SWO (OpenOCD TCP trace port, or a captured-file replay) and pushes decoded events as JSON over a WebSocket on port 7878. ✅ (verified end-to-end: a real WebSocket client receives the decoded `log`/`variable`/`overflow` JSON)
- Validated against real byte streams from a NUCLEO-F446RE. ⚙️ *The pipeline is validated against synthetic and replayed SWO captures in CI; capturing from the physical board requires OpenOCD + the ST-Link + hardware (the SWO command sequence is version-dependent — `nucleus trace --openocd` sends a best-effort setup, and `--replay <file>` plays back a capture).*

The decoder is config-agnostic (raw port + payload bytes); `nucleus-trace` assigns meaning — port 0 reassembles UTF-8 log lines, ports 1–7 decode little-endian typed values named by `[[trace.variables]]` — and broadcasts JSON to every connected client.

---

### Phase 6 — Trace Dashboard

> **Status: ✅ Complete.** The React/Canvas dashboard (`extension/src/dashboard/`) renders logs, live variable charts, and CPU load; it runs identically in the VS Code webview and a standalone browser, and DWT CPU-load decoding landed in `nucleus-trace`.

**Goal:** A polished real-time observability UI.

Scope: the React dashboard in `extension/src/dashboard/`, hosted in a VS Code webview and runnable standalone in a browser. Includes DWT CPU-load decoding (the last packet type).

**Exit criteria:**
- Log panel: decoded port-0 output, timestamped, filterable, searchable. ✅ (plus follow-tail and export-as-text)
- Variable timeline: live Canvas charts for ports 1–7 with auto-scaling Y axis, up to 7 simultaneous variables. ✅ (rolling 30 s window, per-series legend with current values)
- CPU-load strip chart derived from DWT PC-sampling packets. ✅ (`nucleus-trace` emits a rolling `cpuload` event; the dashboard renders a filled strip)
- Dashboard polish: resizable panels, dark mode, export log as text. ✅ (draggable `SplitPane`s, dark/light toggle, connection status + overflow badge)
- Runs identically in the VS Code webview and a standalone browser. ✅ (one esbuild bundle; both connect to `ws://localhost:7878`)

The dashboard is a thin display client: it consumes the daemon's JSON over a WebSocket and assigns no trace meaning of its own (`types.ts`'s `TraceEvent` mirrors the Rust serialization). The extension's `nucleus.openDashboard` command hosts the same bundle in a webview. The TypeScript/React is type-checked (`tsc`) and bundled (esbuild) outside the Rust `make check` gate.

---

### Phase 7 — Distribution + Release Automation

> **Status: ✅ Complete** (the first live publish is a maintainer action needing registry tokens — see note). The crates are publish-ready, the release workflow is wired end-to-end, the reusable action ships, and docs/licensing are in place.

**Goal:** Any stranger can install Nucleus in one command, and releases ship themselves.

Scope: packaging, publishing, and GitHub Actions release automation — the headline outcome of the project.

**Exit criteria:**
- `cargo install nucleus-cli` installs a working CLI (published to crates.io). ✅ Publish-ready — every crate carries the required metadata (versioned path deps, keywords, categories, readme) and `cargo publish --dry-run` packages cleanly; the release workflow runs the publish in dependency order. ⚙️ *The actual first upload requires the `CARGO_REGISTRY_TOKEN` secret (a maintainer step).*
- The extension is published and installable from the VS Code Marketplace. ✅ The release workflow packages a `.vsix` and publishes it. ⚙️ *Marketplace publish needs the `VSCE_PAT` secret.*
- A GitHub Actions release workflow triggers on a version tag and builds cross-platform CLI binaries (Linux/macOS/Windows, x86_64 + arm64) with checksums, publishes the crates, and packages + uploads the `.vsix`. ✅ (`.github/workflows/release.yml`)
- Releases follow semver and ship with a generated changelog. ✅ (`generate_release_notes` + a curated `CHANGELOG.md`)
- A reusable `nucleus-action` runs `nucleus check` + `nucleus build` and posts a PR summary (conflict count, firmware size); a copy-paste `nucleus.yml` is documented. ✅ (`.github/actions/nucleus/`, documented in [`docs/ci.md`](docs/ci.md))

Publishing steps are **secret-gated**, so the workflow is safe to run on forks and tags without leaking or failing — they simply no-op until the tokens are configured. See [`docs/installation.md`](docs/installation.md) for how to install today (from source) and after the first release.

---

### Phase 8 — Docs, Generality Proof + Community Launch

**Goal:** Production-quality, documented, and proven to generalize beyond one chip.

Scope: documentation, a second MCU family (STM32F411RE), and public launch.

**Exit criteria:**
- mdBook docs site published on GitHub Pages, including the firmware integration guide (enabling ITM in OpenOCD, the four lines of C).
- STM32F411RE (NUCLEO-F411RE) supported end-to-end via `family = "STM32F411RE"` and `nucleus init --board NUCLEO-F411RE`, validating that the database design generalizes to a second MCU.
- CI runs `check` + `build` + `test` on every PR; CONTRIBUTING guide and issue templates in place.
- Demo video recorded; public launch (Show HN, r/stm32, r/embedded, STM32/Embedded.fm Discords, Awesome Embedded).

---

## Known Hard Problems

These will consume more time than expected. Budget for them:

### 1. STM32 metadata normalization (biggest time sink)
CMSIS Pack XML data is inconsistent across MCU families. Peripheral names don't always match HAL function names. Some alternate function entries have typos or are missing. You will need:
- A normalization pass over raw pack data
- A hand-maintained patch table for known errors
- Extensive unit testing against real HAL init code

Plan for this consuming 30–40% of Month 1.

### 2. OpenOCD SWO configuration
OpenOCD's SWO support varies by version and probe type. The telnet command sequence to enable SWO capture is undocumented and differs between ST-Link V2 and ST-Link V3. Test on the actual NUCLEO-F446RE board early — don't assume it works until you've seen bytes flowing.

### 3. ITM packet framing edge cases
The CoreSight spec has corner cases around:
- Packets that span WebSocket read boundaries
- Overflow packets (ITM buffer full on device)
- Synchronization packets after a dropped connection

The decoder must handle all of these without panicking. Fuzz-test the decoder with random byte sequences.

### 4. HAL version drift
ST periodically updates their HAL library. The generated `nucleus_init.c` calls HAL functions directly — if ST changes a function signature, the generated code breaks. Mitigate by:
- Testing against a pinned HAL version in CI
- Not generating complex HAL calls — keep it to `Init` functions with config structs only
- Documenting the tested HAL version prominently

---

## Scope Discipline Rules

These are constraints to follow throughout development. Do not negotiate with them mid-project.

1. **One MCU only through Phase 7.** NUCLEO-F446RE. Adding a second family is Phase 8, not earlier.
2. **No DMA collision detection** through the published-toolchain milestone (Phase 7). Ship pin conflict detection first. DMA is complex and low perceived value early.
3. **No full clock tree solver.** Basic clock-domain validation (is the bus enabled?) ships in Phase 2. Full PLL tree solving is a research problem on its own and is not scheduled.
4. **No JetBrains / Neovim extension until after Marketplace launch.** The architecture supports it but build the VS Code extension first.
5. **No cloud registry.** Nucleus is a local tool. No "upload your config to nucleus.dev" features. Keep the attack surface small.
6. **The extension contains zero business logic.** If you're tempted to put constraint checking inside the TypeScript extension, don't. It belongs in the Rust CLI.

---

## Contributing

Contributions are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md) for the
build/test workflow and the architectural rules. Run `make check` (the exact CI
gate) before pushing. Changes ship with tests.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in the work by you shall be dual-licensed
as above, without any additional terms or conditions.

## Naming

"Nucleus" is memorable, short as a CLI command, and has the right systems-programming feel.

**Before committing, check:**
- `crates.io/crates/nucleus` — if taken, use `nucleus-stm32` or `nucleusstm32`
- VS Code Marketplace search for "Nucleus"
- GitHub organization name availability

If the name is blocked, strong alternatives: `stforge`, `cubefree`, `pinsmith`, `tracenow`.

---

## Rust Workspace Structure

```
nucleus/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── nucleus-cli/            # binary crate — CLI entry point, command dispatch
│   ├── nucleus-compiler/       # lib crate — toml parser, constraint solver, codegen
│   ├── nucleus-db/             # lib crate — STM32 constraint database (built from CMSIS packs)
│   ├── nucleus-lsp/            # lib crate — tower-lsp server, wraps nucleus-compiler
│   ├── nucleus-itm/            # lib crate — ITM/CoreSight packet decoder
│   └── nucleus-trace/          # lib crate — OpenOCD integration, WebSocket server
├── extension/                  # VS Code extension (TypeScript)
│   ├── src/
│   │   ├── extension.ts        # activate/deactivate, LSP client setup
│   │   ├── tracePanel.ts       # Webview panel host
│   │   └── dashboard/          # React app (bundled by esbuild)
│   │       ├── App.tsx
│   │       ├── LogPanel.tsx
│   │       └── VariableChart.tsx
│   └── package.json
├── xtask/                      # cargo xtask — build scripts, pack gen, CI helpers
├── tests/
│   ├── fixtures/               # sample stm32.toml files for integration tests
│   └── integration/            # end-to-end tests against known conflict scenarios
└── docs/                       # mdBook documentation source
```

---

## Resume Line

> *"Built Nucleus, an open source VS Code extension + Rust CLI for STM32 development — implements a hardware constraint solver with pin/AF conflict detection against STM32 CMSIS databases, an LSP server exposing live diagnostics in VS Code, and a real-time ARM CoreSight ITM packet decoder streaming structured telemetry to a React dashboard over WebSocket."*

---

## What This Demonstrates (Domain Map)

| Domain | What Nucleus shows you can do |
|---|---|
| Embedded systems | STM32 peripheral model, OpenOCD integration, ARM CoreSight SWO |
| Compiler engineering | Parser, constraint solver, typed IR, code generator |
| Systems programming | Rust, binary protocol decoding, async daemon |
| Language tooling | LSP implementation, VS Code extension development |
| Frontend engineering | React, Canvas API for real-time charting |
| DevOps / CI | GitHub Actions, reproducible embedded builds |
| Software architecture | Multi-crate workspace, CLI-first design, thin UI layer |

No other single project in a typical undergraduate embedded portfolio touches all seven.

---

*Last updated: May 2026. Target hardware: NUCLEO-F446RE. Target completion: August 2026.*
