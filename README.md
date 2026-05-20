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
  - [Phase 1 — Pinmux MVP](#phase-1--pinmux-mvp-month-1)
  - [Phase 2 — SWO Logging](#phase-2--swo-logging-month-2)
  - [Phase 3 — Full Trace Dashboard](#phase-3--full-trace-dashboard-month-3)
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
| `nucleus init` | Scaffold a new STM32 project: `stm32.toml`, `CMakeLists.txt`, `src/main.c`, `.github/workflows/ci.yml` |
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

#### What the compiler deliberately skips (Phase 1)

- DMA channel collision detection — hard, device-specific, low perceived value early on
- Full clock tree validation — complex, defer to Phase 2
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
- **DWT (Data Watchpoint and Trace) packets:** CPU cycle counter data, used for load estimation. Phase 3 only.

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

**CPU load panel** — derived from DWT cycle counter packets, showing approximate CPU utilization as a rolling strip chart. Phase 3 only.

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

Scope discipline is the biggest risk on this project. Each phase ships something real and usable before the next begins.

---

### Phase 1 — Pinmux MVP (Month 1)

**Goal: A developer can replace CubeMX pin config with `stm32.toml` for the NUCLEO-F446RE.**

**Target hardware: NUCLEO-F446RE only. No other board until Phase 3.**

#### Week 1 — Constraint database
- Download STM32F446 CMSIS Pack from ST website
- Write a Rust script to parse the alternate function XML tables into a compiled-in database (a `build.rs` that embeds the data at compile time)
- Cover all GPIOs (PA0–PC15), all alternate functions AF0–AF15, peripheral-to-pin mappings
- Unit test: given pin PA7, assert AF5 = SPI1_MOSI

#### Week 2 — Parser + conflict solver
- Implement `stm32.toml` parser using the `toml` crate with typed structs
- Implement conflict detection: pin collision, AF mismatch, missing required pins, clock domain disabled
- `nucleus check` command: reads `stm32.toml`, prints conflicts with pin names and descriptions, exits non-zero if any error
- Integration test: a `stm32.toml` with a deliberate PA5 collision should produce exactly one error

#### Week 3 — Code generator
- Emit `nucleus_init.c` and `nucleus_init.h`
- Generated code calls standard ST HAL functions (`HAL_UART_Init`, `HAL_SPI_Init` etc.) with resolved parameters — does NOT reimplement HAL
- Test on a real NUCLEO-F446RE: generated init code compiles and initializes peripherals correctly

#### Week 4 — LSP server + VS Code extension skeleton
- Implement LSP server using `tower-lsp`: textDocument/diagnostic (conflicts as squiggles), textDocument/hover (pin info)
- VS Code extension: activate on `stm32.toml` open, spawn `nucleus lsp`, connect LSP client
- Demo: open a `stm32.toml` with a pin conflict, see a red squiggle appear in VS Code

**Phase 1 deliverable: publish extension to VS Code Marketplace. Get first external users.**

---

### Phase 2 — SWO Logging (Month 2)

**Goal: A developer can replace UART `printf` debugging with ITM port 0 logging, visible in VS Code.**

#### Week 5 — OpenOCD integration + ITM packet decoder
- Connect to OpenOCD over telnet, send commands to configure SWO capture
- Implement ARM CoreSight ITM packet decoder in Rust from the architecture spec
- Decode SWIT packets on port 0 as UTF-8 strings
- Unit test packet decoder with known byte sequences from the spec

#### Week 6 — WebSocket server + log panel
- `nucleus trace` command: starts ITM daemon, opens VS Code webview panel
- WebSocket server on port 7878 pushes decoded log messages as JSON
- React log panel in VS Code Webview: timestamped messages, filterable by content
- Test on real hardware: firmware calls `nucleus_log("hello")`, message appears in VS Code

#### Week 7 — Variable tracing (ports 1–7)
- Decode SWIT packets on ports 1–7 as typed values (f32, u16, u32, i32)
- Match port numbers to variable names declared in `[trace.variables]`
- Push typed variable events over WebSocket
- React variable timeline panel: live Canvas chart, auto-scaling Y axis

#### Week 8 — Polish + docs
- Write firmware integration guide (how to enable ITM in OpenOCD config, what 4 lines of C to add)
- Write full project README
- Record demo video: edit `stm32.toml` → see squiggles → fix config → flash firmware → live variable chart in VS Code

---

### Phase 3 — Full Trace Dashboard + CI Story (Month 3)

**Goal: A complete CI pipeline for STM32 projects + CPU load visualization + expand to one more MCU family.**

#### Week 9 — GitHub Actions integration
- `nucleus-action`: a GitHub Action that runs `nucleus check`, builds firmware with `nucleus build`, and posts a summary comment on PRs showing: resolved pin assignments, conflict count, firmware binary size
- Write a sample `.github/workflows/nucleus.yml` that people can copy
- Test on a real repository with deliberate conflicts to verify PR annotations

#### Week 10 — DWT CPU load + dashboard polish
- Decode DWT cycle counter packets (PC sampling)
- Compute rolling CPU utilization estimate
- Add CPU load strip chart to the dashboard
- Dashboard layout polish: resizable panels, dark mode, export log as text file

#### Week 11 — Second MCU (STM32L476RG / NUCLEO-L476RG)
- Extend constraint database to cover STM32L476RG
- Validate that the architecture generalizes (this is the real test of the database design)
- Add `family = "STM32L476RG"` support to `stm32.toml`

#### Week 12 — Release + community
- Write full documentation site (mdBook or Docusaurus, hosted on GitHub Pages)
- Post to: r/stm32, r/embedded, STM32 Discord, Embedded.fm Discord, Hacker News Show HN
- Submit to Awesome Embedded list on GitHub
- Write a technical blog post: "Implementing ARM CoreSight ITM packet decoding from scratch"

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

1. **One MCU only for Phase 1 and 2.** NUCLEO-F446RE. Adding a second family is Phase 3 Week 11, not earlier.
2. **No DMA collision detection until Phase 3.** Ship pin conflict detection first. DMA is complex and low perceived value early.
3. **No clock tree solver until Phase 2.** Basic clock domain validation (is the bus enabled?) in Phase 1. Full PLL tree solving is a research problem on its own.
4. **No JetBrains / Neovim extension until after Marketplace launch.** The architecture supports it but build the VS Code extension first.
5. **No cloud registry.** Nucleus is a local tool. No "upload your config to nucleus.dev" features. Keep the attack surface small.
6. **The extension contains zero business logic.** If you're tempted to put constraint checking inside the TypeScript extension, don't. It belongs in the Rust CLI.

---

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