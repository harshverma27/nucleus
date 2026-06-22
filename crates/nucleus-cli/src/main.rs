//! The `nucleus` command-line interface — command parsing and dispatch.
//!
//! The full surface is live: `check` validates an `stm32.toml`, `route`
//! auto-assigns pins for peripherals declared without them, `init` scaffolds
//! a project, `build` generates HAL init code and drives the cross toolchain,
//! `flash` programs the board, `lsp` starts the language server over stdio, and
//! `trace` decodes ITM/SWO and streams events over a WebSocket.

mod firmware;
mod scaffold;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nucleus_compiler::{check_family, route_family, test_plan, ParseError, Severity};
use nucleus_hil::backend::{Backend, FirmwareArtifact, HilError};
use nucleus_hil::hardware::HardwareBackend;
use nucleus_hil::qemu::QemuBackend;
use nucleus_hil::{run_tests, TestStatus};

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
    /// Auto-route peripheral instances declared without pins; writes a
    /// fully-specified stm32.toml (the M4 constraint auto-router).
    Route {
        /// Path to the config file (defaults to ./stm32.toml).
        #[arg(default_value = "stm32.toml")]
        path: PathBuf,
        /// Write the routed config to this path instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run [[test]] assertions against one or both HIL backends (the M5
    /// substrate); exits non-zero if any selected test fails on any selected
    /// backend.
    Test {
        /// Project root containing stm32.toml and build/firmware.{elf,bin}
        /// (defaults to the current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run on only one backend instead of both.
        #[arg(long, value_enum)]
        backend: Option<BackendArg>,
        /// Run only the test with this name instead of all of them.
        #[arg(long)]
        test: Option<String>,
    },
}

/// clap-friendly mirror of [`nucleus_compiler::BackendSelect`] for the
/// `--backend` flag; kept distinct since `value_enum` wants its own type.
#[derive(Clone, Copy, clap::ValueEnum)]
enum BackendArg {
    Qemu,
    Hardware,
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
        Command::Route { path, out } => run_route(&path, out.as_deref()),
        Command::Test {
            path,
            backend,
            test,
        } => run_test(&path, backend, test.as_deref()),
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

/// Auto-route peripheral instances declared without pins, writing a
/// fully-specified stm32.toml. Exit code: `0` on a successful route, `1` on
/// a parse error or an unroutable config (no output is written on failure).
fn run_route(path: &Path, out: Option<&Path>) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let (report, family_warning) = match route_family(&text) {
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

    match report.outcome {
        Ok(rendered) => {
            match out {
                Some(out_path) => {
                    if let Err(err) = std::fs::write(out_path, &rendered) {
                        eprintln!("error: cannot write {}: {err}", out_path.display());
                        return ExitCode::FAILURE;
                    }
                    println!("{} routed -> {}", path.display(), out_path.display());
                }
                None => print!("{rendered}"),
            }
            ExitCode::SUCCESS
        }
        Err(conflicts) => {
            let n = conflicts.len();
            eprintln!(
                "{}: could not route ({n} conflict{}):\n",
                path.display(),
                if n == 1 { "" } else { "s" }
            );
            for conflict in &conflicts {
                let prefix = match conflict.severity() {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                };
                eprintln!("  {prefix}: {conflict}");
            }
            eprintln!();
            ExitCode::FAILURE
        }
    }
}

/// Backend labels a scripted test should actually run on: the intersection
/// of what `--backend filter` allows and what the test's declared `select`
/// permits. `filter = None` allows both; `select = Both` declares both.
/// Empty result means skip — mirrors `nucleus_hil::backend_selected`'s
/// skip-on-mismatch semantics so `--backend` filters rather than overrides.
/// Order is deterministic: qemu before hardware.
fn scripted_backend_labels(
    filter: Option<BackendArg>,
    select: nucleus_compiler::BackendSelect,
) -> Vec<&'static str> {
    use nucleus_compiler::BackendSelect;

    let filter_allows_qemu = !matches!(filter, Some(BackendArg::Hardware));
    let filter_allows_hardware = !matches!(filter, Some(BackendArg::Qemu));
    let select_allows_qemu = matches!(select, BackendSelect::Both | BackendSelect::Qemu);
    let select_allows_hardware = matches!(select, BackendSelect::Both | BackendSelect::Hardware);

    let mut labels = Vec::new();
    if filter_allows_qemu && select_allows_qemu {
        labels.push("qemu");
    }
    if filter_allows_hardware && select_allows_hardware {
        labels.push("hardware");
    }
    labels
}

/// Run `[[test]]` assertions from `path`'s `stm32.toml` against one or both
/// HIL backends. Exit code: `0` if every selected test passes or is skipped
/// on every backend it ran on, `1` on a parse/conflict error, an unknown
/// `--test` name, missing firmware artifacts, or any failed test.
fn run_test(
    path: &Path,
    backend_filter: Option<BackendArg>,
    test_filter: Option<&str>,
) -> ExitCode {
    let toml_path = path.join("stm32.toml");
    let text = match std::fs::read_to_string(&toml_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", toml_path.display());
            return ExitCode::FAILURE;
        }
    };

    let mut plan = match test_plan(&text) {
        Err(err) => {
            print_parse_error(&toml_path, &err);
            return ExitCode::FAILURE;
        }
        Ok(Err(conflicts)) => {
            let n = conflicts.len();
            eprintln!(
                "{}: {n} conflict{} found:\n",
                toml_path.display(),
                if n == 1 { "" } else { "s" }
            );
            for conflict in &conflicts {
                let prefix = match conflict.severity() {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                };
                eprintln!("  {prefix}: {conflict}");
            }
            eprintln!();
            return ExitCode::FAILURE;
        }
        Ok(Ok(plan)) => plan,
    };

    if plan.is_empty() {
        println!("{}: no [[test]] blocks defined.", path.display());
        return ExitCode::SUCCESS;
    }

    if let Some(name) = test_filter {
        plan.retain(|t| t.name == name);
        if plan.is_empty() {
            eprintln!("error: no test named {name:?}");
            return ExitCode::FAILURE;
        }
    }

    use nucleus_compiler::TestBody;
    let (scripted, declarative): (Vec<_>, Vec<_>) = plan
        .iter()
        .cloned()
        .partition(|t| matches!(t.body, TestBody::Scripted { .. }));

    let mut any_failed = false;

    if !declarative.is_empty() {
        let elf = path.join("build/firmware");
        let bin = path.join("build/firmware.bin");
        if !elf.exists() || !bin.exists() {
            eprintln!(
                "error: {} not found. Run `nucleus build` first.",
                bin.display()
            );
            return ExitCode::FAILURE;
        }
        let firmware = FirmwareArtifact { elf, bin };

        let check_report = match nucleus_compiler::check(&text) {
            Ok(report) => report,
            Err(err) => {
                print_parse_error(&toml_path, &err);
                return ExitCode::FAILURE;
            }
        };

        let mut backends: Vec<Box<dyn Backend>> = Vec::new();
        match backend_filter {
            None => {
                backends.push(Box::new(QemuBackend::default()));
                backends.push(Box::new(HardwareBackend::default()));
            }
            Some(BackendArg::Qemu) => backends.push(Box::new(QemuBackend::default())),
            Some(BackendArg::Hardware) => backends.push(Box::new(HardwareBackend::default())),
        }

        for mut backend in backends {
            let kind = backend.name();
            match backend.start(&firmware, &check_report) {
                Err(err @ HilError::ToolMissing(_)) => {
                    println!("skipped: {kind:?} ({err})");
                }
                Err(err) => {
                    eprintln!("error: {kind:?} failed to start: {err}");
                    any_failed = true;
                }
                Ok(()) => {
                    let outcomes = run_tests(backend.as_mut(), &declarative);
                    let _ = backend.finish();
                    for outcome in &outcomes {
                        let status_icon = match outcome.status {
                            TestStatus::Passed => "PASS",
                            TestStatus::Failed => "FAIL",
                            TestStatus::Skipped => "SKIP",
                        };
                        println!(
                            "  {status_icon} {} [{kind:?}]: {}",
                            outcome.name, outcome.detail
                        );
                        if outcome.status == TestStatus::Failed {
                            any_failed = true;
                        }
                    }
                }
            }
        }
    }

    for test in &scripted {
        let TestBody::Scripted { script } = &test.body else {
            continue;
        };
        // Which backend labels to run this scripted test against: the
        // intersection of what `--backend` allows and what the test's
        // declared `backend` field selects. Mirrors the declarative path's
        // skip-on-mismatch semantics (`nucleus_hil::backend_selected`) —
        // `--backend` is a filter, not an override, so a `backend = "qemu"`
        // test is never force-run on hardware.
        let labels = scripted_backend_labels(backend_filter, test.backend);
        if labels.is_empty() {
            println!(
                "  SKIP {} [scripted]: not selected for the requested backend",
                test.name
            );
            continue;
        }
        for label in labels {
            let status = std::process::Command::new("cargo")
                .args(["test", script, "--", "--exact", "--nocapture"])
                .current_dir(path)
                .env("NUCLEUS_TEST_BACKEND", label)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("  PASS {} [{label}] (scripted)", test.name);
                }
                Ok(_) => {
                    println!("  FAIL {} [{label}] (scripted)", test.name);
                    any_failed = true;
                }
                Err(e) => {
                    eprintln!("  error: cargo test failed to launch: {e}");
                    any_failed = true;
                }
            }
        }
    }

    if any_failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod scripted_backend_tests {
    use super::*;
    use nucleus_compiler::BackendSelect;

    #[test]
    fn no_filter_and_both_runs_qemu_then_hardware() {
        assert_eq!(
            scripted_backend_labels(None, BackendSelect::Both),
            vec!["qemu", "hardware"]
        );
    }

    #[test]
    fn hardware_filter_with_qemu_only_test_skips() {
        // Previously-broken case: a `backend = "qemu"` scripted test must
        // not be force-run when `--backend hardware` is passed.
        assert_eq!(
            scripted_backend_labels(Some(BackendArg::Hardware), BackendSelect::Qemu),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn qemu_filter_with_both_test_narrows_to_qemu() {
        assert_eq!(
            scripted_backend_labels(Some(BackendArg::Qemu), BackendSelect::Both),
            vec!["qemu"]
        );
    }

    #[test]
    fn no_filter_with_hardware_only_test_runs_hardware() {
        assert_eq!(
            scripted_backend_labels(None, BackendSelect::Hardware),
            vec!["hardware"]
        );
    }
}
