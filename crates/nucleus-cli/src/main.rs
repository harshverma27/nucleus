//! The `nucleus` command-line interface.
//!
//! Phases 2–3 implement the config→firmware path: `check` validates an
//! `stm32.toml`, `init` scaffolds a project, `build` generates HAL init code and
//! drives the cross toolchain, and `flash` programs the board. The remaining
//! subcommands (`trace`, `lsp`) are declared so the surface is stable but land
//! in later phases.

mod firmware;
mod scaffold;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nucleus_compiler::{check_family, ParseError};

use scaffold::Written;

/// CLI-first STM32 developer platform.
#[derive(Parser)]
#[command(name = "nucleus", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate stm32.toml against the constraint database and print conflicts.
    Check {
        /// Path to the config file (defaults to ./stm32.toml).
        #[arg(default_value = "stm32.toml")]
        path: PathBuf,
    },
    /// Scaffold a new STM32 project (stm32.toml, CMake, main.c, CI workflow).
    Init {
        /// Directory to scaffold into (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Generate HAL init code and build firmware (.elf/.bin) via CMake.
    Build {
        /// Project root containing stm32.toml (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Flash the built firmware to a connected board with st-flash.
    Flash {
        /// Project root containing build/firmware.bin (defaults to current dir).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Start the ITM trace daemon and dashboard. (Phase 5)
    Trace,
    /// Start the language server (used by the editor extension). (Phase 4)
    Lsp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { path } => run_check(&path),
        Command::Init { path } => run_init(&path),
        Command::Build { path } => firmware::build(&path),
        Command::Flash { path } => firmware::flash(&path),
        Command::Trace => not_yet("trace", "Phase 5"),
        Command::Lsp => not_yet("lsp", "Phase 4"),
    }
}

fn not_yet(name: &str, phase: &str) -> ExitCode {
    eprintln!("nucleus {name}: not implemented yet (scheduled for {phase})");
    ExitCode::FAILURE
}

/// Scaffold a new project under `path`.
fn run_init(path: &Path) -> ExitCode {
    match scaffold::scaffold(path) {
        Ok(results) => {
            let mut created = 0;
            for r in &results {
                match r {
                    Written::Created(p) => {
                        println!("  created  {p}");
                        created += 1;
                    }
                    Written::Skipped(p) => println!("  skipped  {p} (exists)"),
                }
            }
            println!(
                "\nScaffolded {created} file(s). Next: `nucleus check`, then `nucleus build`."
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: failed to scaffold project: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Read, parse, and validate `path`, printing a human report. Exit code:
/// `0` when the config is conflict-free, `1` on conflicts or any read/parse error.
fn run_check(path: &Path) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let (report, family_warning) = match check_family(&text) {
        Ok(result) => result,
        Err(err) => {
            print_parse_error(path, &err);
            return ExitCode::FAILURE;
        }
    };

    if let Some(warning) = &family_warning {
        eprintln!("warning: {warning}");
        eprintln!("         validating against STM32F446RE; results may be inaccurate.\n");
    }

    if report.is_ok() {
        println!(
            "{} OK — {} peripheral(s), no conflicts.",
            path.display(),
            report.config.peripherals.len()
        );
        // A family warning is advisory, not fatal: still exit 0.
        return ExitCode::SUCCESS;
    }

    let n = report.conflicts.len();
    eprintln!(
        "{}: {n} conflict{} found:\n",
        path.display(),
        if n == 1 { "" } else { "s" }
    );
    for conflict in &report.conflicts {
        eprintln!("  error: {conflict}");
    }
    eprintln!();
    ExitCode::FAILURE
}

fn print_parse_error(path: &Path, err: &ParseError) {
    eprintln!("error: {}: {err}", path.display());
}
