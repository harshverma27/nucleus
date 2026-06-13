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
