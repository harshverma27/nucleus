//! `nucleus build` and `nucleus flash`: codegen + toolchain orchestration.
//!
//! `build` validates the config, generates `nucleus_config.h` / `nucleus_init.c`
//! into `src/generated/`, then drives CMake + `arm-none-eabi-gcc`. Codegen runs
//! (and is observable) even when the cross toolchain is absent; only the compile
//! step needs it, and a missing tool produces a clear, actionable error.
//!
//! `flash` programs the board's `firmware.bin` with `st-flash`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use nucleus_compiler::check_family;
use nucleus_db::Database;

/// Result of [`generate_sources`]: where the two generated files landed.
pub struct GeneratedPaths {
    pub config_h: PathBuf,
    pub init_c: PathBuf,
}

/// Validate `stm32.toml` and write the generated sources under `root`. Returns
/// `Err` with a printed message already emitted on failure.
pub fn generate_sources(root: &Path) -> Result<GeneratedPaths, ()> {
    let toml_path = root.join("stm32.toml");
    let text = match std::fs::read_to_string(&toml_path) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", toml_path.display());
            return Err(());
        }
    };

    let (report, family_warning) = match check_family(&text) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("error: {}: {err}", toml_path.display());
            return Err(());
        }
    };
    if let Some(w) = &family_warning {
        eprintln!("warning: {w}\n");
    }
    if !report.is_ok() {
        eprintln!(
            "error: refusing to generate code — {} conflict(s) in stm32.toml. Run `nucleus check`.",
            report.conflicts.len()
        );
        return Err(());
    }

    let db = nucleus_compiler::database_for(&report.config.device.family)
        .unwrap_or_else(|_| Database::f446re());
    let generated = nucleus_compiler::generate(&report.config, &db);

    let dir = root.join("src/generated");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {err}", dir.display());
        return Err(());
    }
    let config_h = dir.join("nucleus_config.h");
    let init_c = dir.join("nucleus_init.c");
    if let Err(err) = std::fs::write(&config_h, &generated.config_h) {
        eprintln!("error: cannot write {}: {err}", config_h.display());
        return Err(());
    }
    if let Err(err) = std::fs::write(&init_c, &generated.init_c) {
        eprintln!("error: cannot write {}: {err}", init_c.display());
        return Err(());
    }

    Ok(GeneratedPaths { config_h, init_c })
}

/// `nucleus build`: generate sources, then configure and build with CMake.
pub fn build(root: &Path) -> ExitCode {
    let paths = match generate_sources(root) {
        Ok(p) => p,
        Err(()) => return ExitCode::FAILURE,
    };
    println!("generated {}", paths.config_h.display());
    println!("generated {}", paths.init_c.display());

    if !tool_available("cmake") {
        eprintln!(
            "error: `cmake` not found on PATH. Install CMake and the \
             arm-none-eabi-gcc toolchain to build firmware."
        );
        return ExitCode::FAILURE;
    }

    let build_dir = root.join("build");
    let configure = run(Command::new("cmake")
        .arg("-S")
        .arg(root)
        .arg("-B")
        .arg(&build_dir));
    if !matches!(configure, Some(true)) {
        eprintln!(
            "error: CMake configure failed. Ensure arm-none-eabi-gcc is installed \
             and STM32CUBE_PATH points at a STM32CubeF4 checkout."
        );
        return ExitCode::FAILURE;
    }

    if matches!(
        run(Command::new("cmake").arg("--build").arg(&build_dir)),
        Some(true)
    ) {
        println!("build OK — firmware in {}", build_dir.display());
        ExitCode::SUCCESS
    } else {
        eprintln!("error: firmware build failed.");
        ExitCode::FAILURE
    }
}

/// `nucleus flash`: program `build/firmware.bin` with `st-flash`.
pub fn flash(root: &Path) -> ExitCode {
    let bin = root.join("build/firmware.bin");
    if !bin.exists() {
        eprintln!(
            "error: {} not found. Run `nucleus build` first.",
            bin.display()
        );
        return ExitCode::FAILURE;
    }
    if !tool_available("st-flash") {
        eprintln!(
            "error: `st-flash` not found on PATH. Install stlink-tools, or flash \
             manually with OpenOCD."
        );
        return ExitCode::FAILURE;
    }
    match run(Command::new("st-flash")
        .arg("write")
        .arg(&bin)
        .arg("0x08000000"))
    {
        Some(true) => {
            println!("flashed {}", bin.display());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("error: st-flash failed.");
            ExitCode::FAILURE
        }
    }
}

/// Run `cmd`, inheriting stdio. `Some(true)` on exit 0, `Some(false)` on a
/// non-zero exit, `None` if the binary could not be spawned.
fn run(cmd: &mut Command) -> Option<bool> {
    match cmd.status() {
        Ok(status) => Some(status.success()),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => {
            eprintln!("error: failed to run command: {err}");
            Some(false)
        }
    }
}

/// Whether `tool` can be spawned (i.e. exists on PATH).
fn tool_available(tool: &str) -> bool {
    match Command::new(tool).arg("--version").output() {
        Ok(_) => true,
        Err(err) if err.kind() == ErrorKind::NotFound => false,
        // Exists but errored on --version: still present.
        Err(_) => true,
    }
}
