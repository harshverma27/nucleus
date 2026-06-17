//! The `nucleus` command-line interface — command parsing and dispatch.
//!
//! The full surface is live: `check` validates an `stm32.toml`, `init` scaffolds
//! a project, `build` generates HAL init code and drives the cross toolchain,
//! `flash` programs the board, `lsp` starts the language server over stdio, and
//! `trace` decodes ITM/SWO and streams events over a WebSocket.

mod firmware;
mod scaffold;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nucleus_compiler::{check_family, ParseError, Severity};

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
        /// Target board: NUCLEO-F446RE (default) or NUCLEO-F411RE.
        #[arg(long)]
        board: Option<String>,
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
    /// Decode ITM/SWO trace and stream events over a WebSocket.
    Trace {
        /// Project config to read `[trace.variables]` from.
        #[arg(long, default_value = "stm32.toml")]
        config: PathBuf,
        /// WebSocket port to serve decoded events on.
        #[arg(long, default_value_t = nucleus_trace::DEFAULT_WS_PORT)]
        ws_port: u16,
        /// Replay a captured raw-SWO file instead of reading live trace.
        #[arg(long)]
        replay: Option<PathBuf>,
        /// TCP address OpenOCD streams trace to (`tpiu config internal :PORT`).
        #[arg(long, default_value = "127.0.0.1:3344")]
        trace_tcp: String,
        /// Also send setup commands to OpenOCD's telnet console at this address.
        #[arg(long)]
        openocd: Option<String>,
    },
    /// Start the language server over stdio (spawned by the editor extension).
    Lsp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { path } => run_check(&path),
        Command::Init { path, board } => run_init(&path, board.as_deref()),
        Command::Build { path } => firmware::build(&path),
        Command::Flash { path } => firmware::flash(&path),
        Command::Trace {
            config,
            ws_port,
            replay,
            trace_tcp,
            openocd,
        } => run_trace(&config, ws_port, replay, trace_tcp, openocd),
        Command::Lsp => run_lsp(),
    }
}

/// Start the trace daemon: decode ITM/SWO and stream events over a WebSocket.
fn run_trace(
    config: &Path,
    ws_port: u16,
    replay: Option<PathBuf>,
    trace_tcp: String,
    openocd: Option<String>,
) -> ExitCode {
    use nucleus_trace::{Source, TraceOptions, VariableMap};

    // The variable map and clock settings come from stm32.toml when present;
    // tracing still works (port-0 logs) without it.
    let (variables, cpu_hz, swo_hz) = match std::fs::read_to_string(config) {
        Ok(text) => match nucleus_compiler::config::parse(&text) {
            Ok(cfg) => (
                VariableMap::from_config(&cfg.trace),
                cfg.device.clock_hz.unwrap_or(180_000_000) as u32,
                cfg.trace.swo_freq.unwrap_or(2_000_000) as u32,
            ),
            Err(err) => {
                eprintln!("error: {}: {err}", config.display());
                return ExitCode::FAILURE;
            }
        },
        Err(_) => {
            eprintln!(
                "warning: {} not found; tracing port-0 logs only (no named variables).",
                config.display()
            );
            (VariableMap::new(), 180_000_000, 2_000_000)
        }
    };

    let source = match replay {
        Some(path) => Source::File(path),
        None => Source::Tcp(trace_tcp.clone()),
    };

    // Derive the trace port for OpenOCD setup from the TCP address.
    let openocd = openocd.map(|telnet| {
        let trace_port = trace_tcp
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3344);
        (telnet, trace_port, cpu_hz, swo_hz)
    });

    let opts = TraceOptions {
        ws_addr: format!("127.0.0.1:{ws_port}"),
        source,
        openocd,
        variables,
    };

    match nucleus_trace::run_blocking(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: trace failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Start the language server over stdio. Runs until the editor disconnects.
fn run_lsp() -> ExitCode {
    match nucleus_lsp::run_stdio() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: language server failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Scaffold a new project under `path` for the chosen board.
fn run_init(path: &Path, board: Option<&str>) -> ExitCode {
    let profile = match board {
        None => scaffold::BoardProfile::F446RE,
        Some(name) => match scaffold::BoardProfile::from_board_name(name) {
            Some(p) => p,
            None => {
                eprintln!("error: unknown board {name:?}");
                eprintln!("       supported: NUCLEO-F446RE, NUCLEO-F411RE");
                return ExitCode::FAILURE;
            }
        },
    };

    match scaffold::scaffold(path, &profile) {
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
                "\nScaffolded {created} file(s) for {}. Next: `nucleus check`, then `nucleus build`.",
                profile.board
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
        eprintln!("         falling back to STM32F446RE; results may be inaccurate.\n");
    }

    if report.is_ok() {
        println!(
            "{} OK — {} peripheral(s), no conflicts.",
            path.display(),
            report.config.peripherals.len()
        );
        // Warnings don't fail the build, but still surface them.
        if !report.conflicts.is_empty() {
            eprintln!();
            for conflict in &report.conflicts {
                eprintln!("  warning: {conflict}");
            }
        }
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
        let prefix = match conflict.severity() {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        eprintln!("  {prefix}: {conflict}");
    }
    eprintln!();
    ExitCode::FAILURE
}

fn print_parse_error(path: &Path, err: &ParseError) {
    eprintln!("error: {}: {err}", path.display());
}
