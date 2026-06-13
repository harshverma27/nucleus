# Design: Official STM32F411RE Support

**Date:** 2026-06-13
**Status:** Approved, ready for implementation planning

## Goal

Add the NUCLEO-F411RE as a fully supported second board, end-to-end:
constraint database, `nucleus check`, `nucleus init` scaffolding,
`nucleus build` codegen, and LSP diagnostics/hover/completion. This fulfills
the README's Phase 8 "second MCU family" generality-proof exit criterion.

STM32F411RE is STM32F4 family — same `STM32F4xx_HAL_Driver`, same CMSIS
device family as the already-supported F446RE, both with 512K flash / 128K
RAM, both with 8 MHz HSE from the on-board ST-Link. This makes it a
lower-risk generalization target than a cross-family chip (e.g. the
previously-named STM32L476RG, which uses a different HAL family). The
README's Phase 8 section is updated to name STM32F411RE as the second
family; the L476RG mention is replaced.

## 1. Database layer (`nucleus-db`)

- Vendor two new files from `STMicroelectronics/STM32_open_pin_data`
  (`master`, same source as the existing F446 files):
  - `packdata/STM32F411RETx.xml` — LQFP64 package pinout
  - `packdata/GPIO-STM32F411_gpio_v1_0_Modes.xml` — pin ↔ AF ↔ signal mux table
  - Update `packdata/README.md`'s file table with both entries.
- `build.rs` runs the existing `pack::parse_gpio_modes` /
  `pack::parse_package_pins` / `pack::apply_patches` /
  `pack::generate_table` pipeline a second time over the F411 files,
  emitting a byte-deterministic `f411re_gen.rs` into `$OUT_DIR` (same
  pattern as `f446re_gen.rs`).
- `data.rs` gains:
  - `pub(crate) const F411RE` — `include!`d generated table.
  - `SEED_F411RE` — a small hand-verified subset (datasheet
    DS10314/RM0383 cross-checked), plus a unit test asserting every seed
    entry appears identically in `F411RE` (mirrors the existing F446RE
    seed test).
- `lib.rs`:
  - New `Database::f411re() -> Database` constructor.
  - New `Database::has_peripheral(&self, name: &str) -> bool` — true if any
    entry's `peripheral` field equals `name`. Used by the solver's new
    `PeripheralUnavailable` conflict (section 2) to determine whether a
    configured peripheral exists on the selected MCU at all.
- Any F411-specific pack-data anomalies discovered during implementation are
  recorded in the existing `pack::PATCHES` table (shared across families —
  entries are keyed by exact `(pin, af, peripheral, signal)` so there's no
  cross-family collision risk).

## 2. Compiler layer (`nucleus-compiler`)

- `lib.rs::database_for()`:
  - Adds `"STM32F411RE" => Ok(Database::f411re())`.
  - `UnknownFamily`'s `Display` message updates to name both supported
    families (`STM32F446RE` and `STM32F411RE`).
  - Becomes `pub` so the CLI (`firmware.rs`) and LSP (`analysis.rs`) can
    reuse the same family → DB resolution instead of each hardcoding
    `Database::f446re()`. Both adopt the existing `check_family` fallback
    pattern: unknown/missing family falls back to F446RE.
- `model.rs::peripheral_bus()`: left family-agnostic. F411's RCC
  bus-enable assignments for the peripherals Nucleus models (USART, SPI,
  I2C, TIM) match F446's in the reference manuals (RM0383 vs RM0390). This
  is verified during implementation; if a genuine discrepancy is found for
  a modeled peripheral, the function is parametrized by family at that
  point — not anticipated.
- New conflict variant in `solver.rs`:
  ```rust
  Conflict::PeripheralUnavailable { peripheral: String, family: String }
  ```
  - `Display`: `"peripheral {peripheral} is not available on {family}"`,
    e.g. `"peripheral SPI4 is not available on STM32F411RE"`.
  - In `solve()`, for each configured peripheral instance, before the
    per-role pin checks: if `!db.has_peripheral(&peripheral)`, push this
    conflict and `continue` (skip pin/clock checks for that instance — a
    nonexistent peripheral would otherwise produce confusing spurious
    `AfMismatch`/`MissingPin` errors).
  - `family` is read from `config.device.family` (already in scope in
    `solve`).

## 3. CLI scaffolding (`nucleus-cli`)

- `Command::Init` gains an optional `--board <NAME>` argument
  (`Option<String>`).
  - Accepted values (case-insensitive): `NUCLEO-F446RE` (default if
    omitted — preserves current behavior exactly) and `NUCLEO-F411RE`.
  - An unrecognized value prints an error listing the supported boards and
    exits non-zero.
- New `scaffold::BoardProfile` struct capturing everything that varies
  between boards:
  - `family: &'static str` (`"STM32F446RE"` / `"STM32F411RE"`)
  - `board: &'static str` (`"NUCLEO-F446RE"` / `"NUCLEO-F411RE"`)
  - `mcu_define: &'static str` (`"STM32F446xx"` / `"STM32F411xE"`)
  - `startup_asm: &'static str` (`"startup_stm32f446xx.s"` /
    `"startup_stm32f411xe.s"`)
  - `linker_filename: &'static str` (`"STM32F446RETx_FLASH.ld"` /
    `"STM32F411RETx_FLASH.ld"`)
  - `clock_hz: u32` (`180_000_000` / `100_000_000` — F411's max SYSCLK is
    100 MHz vs F446's 180 MHz)
  - Two consts: `BoardProfile::F446RE`, `BoardProfile::F411RE`.
- Templates with board-specific content become functions taking
  `&BoardProfile` and `format!`-ing the existing const template strings:
  `STM32_TOML`, `CMAKELISTS` (linker script reference, `mcu_define`,
  `startup_asm`), `LINKER_SCRIPT` (filename + header comment only — the
  memory layout body is identical for both boards, both 512K flash / 128K
  RAM), `HAL_CONF_H` (board-name comment).
- Templates with no board-specific content stay untouched `&'static str`
  consts: `main.c`, `stm32f4xx_it.{c,h}`, `cmake/arm-none-eabi-gcc.cmake`,
  `.github/workflows/ci.yml`, `.gitignore`.
- `run_init` resolves `--board` to a `BoardProfile` and passes it into
  `scaffold::scaffold`.
- `firmware.rs`'s hardcoded `Database::f446re()` (currently line 53)
  switches to `nucleus_compiler::database_for(&config.device.family)` with
  F446RE fallback on error — same resolution used by `check`.
- The "validating against STM32F446RE; results may be inaccurate" warning
  in `main.rs::run_check` is reworded to describe the fallback generically
  (it's still F446RE specifically, since that's the fallback DB, but the
  wording shouldn't imply F446RE is the *only* supported family).

## 4. LSP (`nucleus-lsp`)

- `analysis.rs::db()` is currently hardcoded to `Database::f446re()`.
  `diagnostics`/`hover`/`completion` already call `check_family(text)` to
  get the parsed config and family; they now resolve the DB via the same
  shared `database_for` (F446RE fallback) instead of the fixed `db()`.
  This makes diagnostics, hover (pin AF tables), and pin-name completion
  correct for documents with `family = "STM32F411RE"`.

## 5. Testing

- `nucleus-db`:
  - F411RE seed cross-validation test (mirrors the F446RE seed test).
  - `has_peripheral` tests: true for a peripheral present on F411 (e.g.
    `"USART2"`), false for one absent on F411 (e.g. `"SPI4"`, if indeed
    absent — confirmed against the generated table during
    implementation).
- `nucleus-compiler`:
  - `database_for("STM32F411RE")` resolves to `Database::f411re()`.
  - Solver test: a peripheral present on F446 but absent on F411,
    configured under `family = "STM32F411RE"`, produces exactly one
    `PeripheralUnavailable` conflict and no spurious `AfMismatch`/
    `MissingPin` conflicts for that instance.
  - Clean-config solver + codegen test against `Database::f411re()`
    (mirrors the existing F446RE clean-config tests).
- `nucleus-cli`:
  - New fixture `tests/fixtures/f411re_clean.toml` (`family =
    "STM32F411RE"`, a small valid peripheral set).
  - Integration test: `nucleus init --board NUCLEO-F411RE` scaffolds a
    project whose `CMakeLists.txt`, linker filename, and `stm32.toml`
    reflect the F411RE `BoardProfile` (MCU define, startup file, linker
    script name, family/board/clock_hz).
  - Existing F446RE init integration test continues to cover the
    no-flag/default path unchanged.
- `nucleus-lsp`:
  - Analysis test with `family = "STM32F411RE"` verifying hover/diagnostics
    resolve against `Database::f411re()` (e.g. a pin/AF combination that
    differs between F411 and F446).

## 6. Documentation

- `README.md`: Phase 8 section rewritten — STM32F411RE named as the second
  supported family, exit criteria updated to `family = "STM32F411RE"`
  end-to-end via `nucleus init --board NUCLEO-F411RE`; "Target hardware"
  line updated; the STM32L476RG mention is removed/replaced. Other prose
  referencing "F446RE only" gets a light touch-up.
- `packdata/README.md`: add the two new vendored F411 files to the table.
- `docs/cli.md`: document `nucleus init --board`.
- `CHANGELOG.md`: new entry for F411RE support.
- `CLAUDE.md`: Phase 8 status updated to reflect completion, once
  implementation lands (last step of the implementation plan).

## Out of scope

- No new conflict classes beyond `PeripheralUnavailable`.
- No changes to the ITM/trace pipeline (MCU-agnostic already).
- No new CMake/toolchain logic beyond the templated `BoardProfile`
  substitutions — `STM32CUBE_PATH` continues to point at one
  `STM32CubeF4` checkout that covers both F411 and F446 (`STM32F4xx_HAL_Driver`
  + `CMSIS/Device/ST/STM32F4xx` cover both device variants).
- `nucleus flash`/`nucleus trace` are unchanged (already MCU-agnostic).
