//! End-to-end tests for the `nucleus` binary against repo fixtures.
//!
//! These exercise the Phase 2 exit-criterion directly: `nucleus check` must
//! exit non-zero on conflicts (so CI can gate on it) and zero on a clean file,
//! and the deliberate PA5-collision fixture must yield exactly one error.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn run_check(name: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nucleus"))
        .arg("check")
        .arg(fixture(name))
        .output()
        .expect("failed to run nucleus binary")
}

fn run_route(name: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nucleus"))
        .arg("route")
        .arg(fixture(name))
        .output()
        .expect("failed to run nucleus binary")
}

#[test]
fn clean_config_exits_zero() {
    let out = run_check("clean.toml");
    assert!(
        out.status.success(),
        "expected success, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn pa5_collision_exits_nonzero_with_exactly_one_error() {
    let out = run_check("pa5_collision.toml");
    assert!(!out.status.success(), "expected non-zero exit on collision");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let error_lines = stderr.lines().filter(|l| l.contains("error:")).count();
    assert_eq!(
        error_lines, 1,
        "expected exactly one error, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("PA5") && stderr.contains("pin collision"),
        "error should name the colliding pin, got:\n{stderr}"
    );
}

#[test]
fn overclocked_apb1_exits_nonzero_with_clock_constraint() {
    // M1 exit criterion at the CLI boundary: a clock misconfiguration CubeMX
    // accepts is caught, with exactly one error and no spurious conflicts.
    let out = run_check("overclock_apb1.toml");
    assert!(
        !out.status.success(),
        "expected non-zero exit on over-clock"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let error_lines = stderr.lines().filter(|l| l.contains("error:")).count();
    assert_eq!(
        error_lines, 1,
        "expected exactly one error, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("APB1") && stderr.contains("clock constraint") && stderr.contains("45 MHz"),
        "error should name the over-clocked bus, got:\n{stderr}"
    );
}

#[test]
fn dma_collision_exits_nonzero_with_suggestion() {
    // M2 exit criterion at the CLI boundary: two peripherals contending one DMA
    // stream → exactly one error naming both, with a proposed free alternative.
    let out = run_check("dma_collision.toml");
    assert!(
        !out.status.success(),
        "expected non-zero exit on DMA collision"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let error_lines = stderr.lines().filter(|l| l.contains("error:")).count();
    assert_eq!(
        error_lines, 1,
        "expected exactly one error, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("DMA collision")
            && stderr.contains("I2C1")
            && stderr.contains("UART5")
            && stderr.contains("move I2C1"),
        "error should name both peripherals and a suggestion, got:\n{stderr}"
    );
}

#[test]
fn exti_collision_exits_nonzero() {
    // M3 exit criterion at the CLI boundary: two [[exti]] entries (PA0, PB0)
    // both claim EXTI line 0 -> exactly one error naming both pins.
    let out = run_check("exti_collision.toml");
    assert!(
        !out.status.success(),
        "expected non-zero exit on EXTI collision"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let error_lines = stderr.lines().filter(|l| l.contains("error:")).count();
    assert_eq!(
        error_lines, 1,
        "expected exactly one error, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("PA0") && stderr.contains("PB0"),
        "error should name both colliding pins, got:\n{stderr}"
    );
}

#[test]
fn irq_unhandled_exits_nonzero() {
    // M3 exit criterion at the CLI boundary: USART3 doesn't exist on the
    // F411 at all, so this legitimately produces two independent conflicts
    // (PeripheralUnavailable from the pin/AF pass, IrqConflict from
    // irq::validate()'s own pass over `irq = true`). Unlike the
    // single-conflict fixtures above, we don't assert an exact error count
    // here — just that the IRQ-specific conflict fired and named USART3.
    let out = run_check("irq_unhandled.toml");
    assert!(
        !out.status.success(),
        "expected non-zero exit on unhandled IRQ"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("IRQ conflict") && stderr.contains("USART3"),
        "error should name an IRQ conflict on USART3, got:\n{stderr}"
    );
}

#[test]
fn priority_inversion_exits_zero_with_warning() {
    // M3 severity exit criterion at the CLI boundary: a warning-only conflict
    // (dma_priority > irq_priority on the same peripheral) still exits 0, but
    // the warning is printed (prefixed "warning:", not "error:").
    let out = run_check("priority_inversion.toml");
    assert!(
        out.status.success(),
        "expected success despite a warning-only conflict, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let warning_lines = stderr.lines().filter(|l| l.contains("warning:")).count();
    assert_eq!(
        warning_lines, 1,
        "expected exactly one warning, got stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("error:"),
        "a warnings-only run must not print any error: line, got:\n{stderr}"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("OK"),
        "success message should still print, got stdout:\n{stdout}"
    );
}

#[test]
fn pa5_collision_error_lines_are_prefixed_error_not_warning() {
    // Regression: error-severity conflicts must still print "  error: ", not
    // "  warning: ", now that the prefix is severity-driven instead of fixed.
    let out = run_check("pa5_collision.toml");
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("warning:"),
        "an error-only run must not print any warning: line, got:\n{stderr}"
    );
}

#[test]
fn missing_file_exits_nonzero() {
    let out = Command::new(env!("CARGO_BIN_EXE_nucleus"))
        .arg("check")
        .arg("definitely-does-not-exist.toml")
        .output()
        .expect("failed to run nucleus binary");
    assert!(!out.status.success());
}

// ---- Phase 3: init / build ------------------------------------------------

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// A throwaway directory under the system temp dir, removed on drop.
struct TempProject(PathBuf);

impl TempProject {
    fn new() -> TempProject {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nucleus-it-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempProject(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn nucleus(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nucleus"))
        .args(args)
        .output()
        .expect("failed to run nucleus binary")
}

#[test]
fn init_scaffolds_a_buildable_project() {
    let proj = TempProject::new();
    let out = nucleus(&["init".as_ref(), proj.path().as_os_str()]);
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for f in [
        "stm32.toml",
        "CMakeLists.txt",
        "cmake/arm-none-eabi-gcc.cmake",
        "STM32F446RETx_FLASH.ld",
        "src/main.c",
        "src/stm32f4xx_hal_conf.h",
        "src/stm32f4xx_it.h",
        "src/stm32f4xx_it.c",
        ".github/workflows/ci.yml",
    ] {
        assert!(proj.path().join(f).exists(), "missing scaffolded file {f}");
    }
}

#[test]
fn init_is_idempotent_and_skips_existing() {
    let proj = TempProject::new();
    assert!(nucleus(&["init".as_ref(), proj.path().as_os_str()])
        .status
        .success());
    let second = nucleus(&["init".as_ref(), proj.path().as_os_str()]);
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("skipped"), "re-init should skip: {stdout}");
}

#[test]
fn scaffolded_project_passes_check() {
    let proj = TempProject::new();
    assert!(nucleus(&["init".as_ref(), proj.path().as_os_str()])
        .status
        .success());
    let toml = proj.path().join("stm32.toml");
    let out = nucleus(&["check".as_ref(), toml.as_os_str()]);
    assert!(
        out.status.success(),
        "scaffolded config should be valid: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn build_generates_hal_sources() {
    let proj = TempProject::new();
    assert!(nucleus(&["init".as_ref(), proj.path().as_os_str()])
        .status
        .success());

    // `build` may fail at the CMake/cross-compile step if the toolchain isn't
    // installed, but codegen runs first and must always produce the sources.
    let _ = nucleus(&["build".as_ref(), proj.path().as_os_str()]);

    let config_h = proj.path().join("src/generated/nucleus_config.h");
    let init_c = proj.path().join("src/generated/nucleus_init.c");
    assert!(config_h.exists(), "nucleus_config.h not generated");
    assert!(init_c.exists(), "nucleus_init.c not generated");

    let init = std::fs::read_to_string(&init_c).unwrap();
    assert!(init.contains("void Nucleus_Init(void)"));
    assert!(init.contains("HAL_UART_Init(&huart2);"));
    assert!(init.contains("GPIO_AF7_USART2;"));
}

#[test]
fn build_refuses_a_conflicting_config() {
    let proj = TempProject::new();
    // A config whose USART2_TX pin doesn't exist on the F446 (AF mismatch).
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n\n[peripherals.usart2]\ntx = \"PB0\"\nrx = \"PA3\"\n",
    )
    .unwrap();
    let out = nucleus(&["build".as_ref(), proj.path().as_os_str()]);
    assert!(
        !out.status.success(),
        "build should refuse a conflicting config"
    );
    assert!(!proj.path().join("src/generated/nucleus_init.c").exists());
}

// ---- Phase 8: F411RE support -----------------------------------------------

#[test]
fn f411re_fixture_passes_check() {
    let out = run_check("f411re_clean.toml");
    assert!(
        out.status.success(),
        "expected F411RE fixture to pass, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn init_f411re_scaffolds_board_specific_files() {
    let proj = TempProject::new();
    let out = nucleus(&[
        "init".as_ref(),
        proj.path().as_os_str(),
        "--board".as_ref(),
        "NUCLEO-F411RE".as_ref(),
    ]);
    assert!(
        out.status.success(),
        "init --board failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // F411 linker script, not the F446 one.
    assert!(proj.path().join("STM32F411RETx_FLASH.ld").exists());
    assert!(!proj.path().join("STM32F446RETx_FLASH.ld").exists());

    let toml = std::fs::read_to_string(proj.path().join("stm32.toml")).unwrap();
    assert!(toml.contains("STM32F411RE"));
    assert!(toml.contains("NUCLEO-F411RE"));

    let cmake = std::fs::read_to_string(proj.path().join("CMakeLists.txt")).unwrap();
    assert!(cmake.contains("STM32F411xE"));
    assert!(cmake.contains("startup_stm32f411xe.s"));
    assert!(cmake.contains("STM32F411RETx_FLASH.ld"));
}

#[test]
fn init_f411re_project_passes_check_and_builds() {
    let proj = TempProject::new();
    assert!(nucleus(&[
        "init".as_ref(),
        proj.path().as_os_str(),
        "--board".as_ref(),
        "NUCLEO-F411RE".as_ref(),
    ])
    .status
    .success());

    // The scaffolded F411RE config validates against the F411 DB.
    let toml = proj.path().join("stm32.toml");
    assert!(nucleus(&["check".as_ref(), toml.as_os_str()])
        .status
        .success());

    // Codegen runs (the cross-compile may fail without a toolchain).
    let _ = nucleus(&["build".as_ref(), proj.path().as_os_str()]);
    assert!(proj.path().join("src/generated/nucleus_init.c").exists());
}

// ---- M4: `nucleus route` -----------------------------------------------

#[test]
fn route_assigns_pins_and_prints_to_stdout() {
    let proj = TempProject::new();
    let toml = proj.path().join("stm32.toml");
    std::fs::write(&toml, "[peripherals.usart2]\n").unwrap();

    let out = nucleus(&["route".as_ref(), toml.as_os_str()]);
    assert!(
        out.status.success(),
        "expected route to succeed, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tx = \"PA2\"") && stdout.contains("rx = \"PA3\""),
        "expected routed pins in stdout, got:\n{stdout}"
    );
}

#[test]
fn route_writes_to_out_path_and_not_stdout() {
    let proj = TempProject::new();
    let toml = proj.path().join("stm32.toml");
    std::fs::write(&toml, "[peripherals.usart2]\n").unwrap();
    let out_path = proj.path().join("routed.toml");

    let out = nucleus(&[
        "route".as_ref(),
        toml.as_os_str(),
        "--out".as_ref(),
        out_path.as_os_str(),
    ]);
    assert!(
        out.status.success(),
        "expected route to succeed, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("tx = \"PA2\""),
        "routed TOML should go to --out, not stdout, got stdout:\n{stdout}"
    );

    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(
        written.contains("tx = \"PA2\"") && written.contains("rx = \"PA3\""),
        "expected routed pins in {}, got:\n{written}",
        out_path.display()
    );
}

#[test]
fn route_unroutable_config_exits_nonzero_and_writes_no_file() {
    let proj = TempProject::new();
    let toml = proj.path().join("stm32.toml");
    // USART2_TX's only candidate is PA2; pre-occupy it so routing fails.
    std::fs::write(
        &toml,
        "[peripherals.tim5]\nchannel3 = \"PA2\"\n\n[peripherals.usart2]\n",
    )
    .unwrap();
    let out_path = proj.path().join("routed.toml");

    let out = nucleus(&[
        "route".as_ref(),
        toml.as_os_str(),
        "--out".as_ref(),
        out_path.as_os_str(),
    ]);
    assert!(
        !out.status.success(),
        "expected non-zero exit on unroutable config"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error:"),
        "expected an error:-prefixed conflict, got stderr:\n{stderr}"
    );
    assert!(
        !out_path.exists(),
        "no file should be written on a failed route"
    );
}

// ---- M4: golden fixtures for `nucleus route` ---------------------------

#[test]
fn route_simple_fixture_assigns_pins_deterministically() {
    // usart2 + spi1, no pins set: each required role has exactly one
    // uncontended candidate on the F446, so the route is trivial and exact.
    let out = run_route("route_simple.toml");
    assert!(
        out.status.success(),
        "expected route_simple.toml to route, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tx = \"PA2\"")
            && stdout.contains("rx = \"PA3\"")
            && stdout.contains("mosi = \"PA7\"")
            && stdout.contains("miso = \"PA6\"")
            && stdout.contains("sck = \"PA5\""),
        "expected routed pins in stdout, got:\n{stdout}"
    );
}

#[test]
fn route_simple_output_passes_a_chained_check() {
    // Issue #20's explicit "passthrough" acceptance criterion: the routed
    // output is itself a valid stm32.toml. Route route_simple.toml, write its
    // stdout to a tempfile, and feed that path straight into `nucleus check`.
    let routed = run_route("route_simple.toml");
    assert!(routed.status.success());

    let proj = TempProject::new();
    let out_path = proj.path().join("routed.toml");
    std::fs::write(&out_path, &routed.stdout).unwrap();

    let checked = nucleus(&["check".as_ref(), out_path.as_os_str()]);
    assert!(
        checked.status.success(),
        "routed output should pass check, stderr:\n{}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn route_complex_fixture_is_reproducible() {
    // Issue #20's "optimal assignment reproducible" criterion: four
    // uncontended peripheral kinds at once, routed twice, must produce
    // byte-identical output both times.
    let first = run_route("route_complex.toml");
    assert!(
        first.status.success(),
        "expected route_complex.toml to route, stderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = run_route("route_complex.toml");
    assert!(second.status.success());

    assert_eq!(
        first.stdout, second.stdout,
        "routed output must be byte-identical across runs"
    );
}

#[test]
fn route_overconstrained_fixture_exits_nonzero_naming_stuck_role() {
    // tim5.channel3 pre-occupies PA2, USART2_TX's only candidate -> Unroutable.
    let out = run_route("route_overconstrained.toml");
    assert!(
        !out.status.success(),
        "expected non-zero exit on an overconstrained config"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error:")
            && stderr.contains("unroutable [USART2_TX]")
            && stderr.contains("PA2")
            && stderr.contains("tim5.channel3"),
        "error should name the stuck role and what occupies its candidate, got:\n{stderr}"
    );
}

// ---- M6: `nucleus test` -------------------------------------------------

#[test]
fn test_with_no_blocks_succeeds_and_prints_message() {
    let proj = TempProject::new();
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n",
    )
    .unwrap();

    let out = nucleus(&["test".as_ref(), proj.path().as_os_str()]);
    assert!(
        out.status.success(),
        "expected success with no [[test]] blocks, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no [[test]] blocks defined"),
        "expected the no-tests message, got stdout:\n{stdout}"
    );
}

#[test]
fn test_with_invalid_assertion_fails_with_conflict() {
    let proj = TempProject::new();
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n\n[[test]]\nname = \"bogus\"\nassertion = \"this is not a real assertion\"\n",
    )
    .unwrap();

    let out = nucleus(&["test".as_ref(), proj.path().as_os_str()]);
    assert!(
        !out.status.success(),
        "expected failure on an invalid [[test]] assertion"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error:") && stderr.contains("conflict"),
        "expected a printed conflict, got stderr:\n{stderr}"
    );
}

#[test]
fn test_with_unknown_test_name_fails() {
    let proj = TempProject::new();
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n\n[[test]]\nname = \"real_test\"\nassertion = \"pin PA5 is high within 10ms\"\n",
    )
    .unwrap();

    let out = nucleus(&[
        "test".as_ref(),
        proj.path().as_os_str(),
        "--test".as_ref(),
        "does_not_exist".as_ref(),
    ]);
    assert!(
        !out.status.success(),
        "expected failure when --test names a nonexistent test"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no test named"),
        "expected the no-test-named message, got stderr:\n{stderr}"
    );
}

#[test]
fn test_with_missing_firmware_fails_before_touching_a_backend() {
    let proj = TempProject::new();
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n\n[[test]]\nname = \"real_test\"\nassertion = \"pin PA5 is high within 10ms\"\n",
    )
    .unwrap();
    // Deliberately do not run `nucleus build` — build/firmware.bin won't exist.

    let out = nucleus(&["test".as_ref(), proj.path().as_os_str()]);
    assert!(
        !out.status.success(),
        "expected failure when build/firmware.bin is missing"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not found") && stderr.contains("nucleus build"),
        "expected the flash-style 'not found, run nucleus build first' message, got stderr:\n{stderr}"
    );
}

#[test]
fn test_with_valid_block_and_present_firmware_reaches_backend_start() {
    let proj = TempProject::new();
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n\n[[test]]\nname = \"boot_log\"\nassertion = \"trace event \\\"O\\\" within 500ms\"\n",
    )
    .unwrap();
    // Present-but-fake firmware: empty files are enough to pass the
    // existence check in main.rs; an empty ELF will fail to actually boot
    // (or be skipped if the backend's tool isn't installed), but that's not
    // what this test asserts — it only proves the CLI gets past the
    // "run `nucleus build` first" gate and attempts a backend.
    std::fs::create_dir_all(proj.path().join("build")).unwrap();
    std::fs::write(proj.path().join("build/firmware"), b"").unwrap();
    std::fs::write(proj.path().join("build/firmware.bin"), b"").unwrap();

    let out = nucleus(&["test".as_ref(), proj.path().as_os_str()]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Run `nucleus build` first"),
        "expected the firmware-found gate to pass, got stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn lockstep_with_present_firmware_runs_both_backends_concurrently_without_hanging() {
    // Regression test for the two HIL backends being started/collected on
    // their own threads (issue 34) instead of one after another: this must
    // still reach both backends and return without deadlocking or panicking
    // in the join, regardless of which backend's thread finishes first.
    let proj = TempProject::new();
    std::fs::write(
        proj.path().join("stm32.toml"),
        "[device]\nfamily = \"STM32F446RE\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(proj.path().join("build")).unwrap();
    std::fs::write(proj.path().join("build/firmware"), b"").unwrap();
    std::fs::write(proj.path().join("build/firmware.bin"), b"").unwrap();

    let out = nucleus(&["lockstep".as_ref(), proj.path().as_os_str()]);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Run `nucleus build` first"),
        "expected the firmware-found gate to pass, got stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn init_rejects_unknown_board() {
    let proj = TempProject::new();
    let out = nucleus(&[
        "init".as_ref(),
        proj.path().as_os_str(),
        "--board".as_ref(),
        "NUCLEO-H750".as_ref(),
    ]);
    assert!(!out.status.success(), "unknown board should exit non-zero");
    assert!(!proj.path().join("stm32.toml").exists());
}
