//! The Nucleus pinmux compiler.
//!
//! Owns the `stm32.toml` → diagnostics pipeline: [`config`] parses the file,
//! [`solver`] validates it against the [`nucleus_db`] constraint database, and
//! [`check`] is the one-call entry point the CLI and (later) the LSP build on.
//!
//! Phase 2 ships the parser and the constraint solver (four conflict classes).
//! HAL code generation lands in Phase 3.

pub mod clocks;
pub mod codegen;
pub mod config;
pub mod dma;
pub mod irq;
pub mod model;
pub mod solver;

use nucleus_db::clock::ClockTree;
use nucleus_db::dma::DmaMap;
use nucleus_db::irq::IrqMap;
use nucleus_db::Database;

pub use clocks::{PeripheralFreq, ResolvedClocks};
pub use codegen::{generate, Generated};
pub use config::{Config, ParseError};
pub use solver::Conflict;

/// The outcome of checking one `stm32.toml`.
#[derive(Debug, Clone)]
pub struct CheckReport {
    /// The parsed config (useful to callers that go on to codegen).
    pub config: Config,
    /// All detected conflicts, in deterministic order. Empty means the config
    /// is valid.
    pub conflicts: Vec<Conflict>,
}

impl CheckReport {
    /// Whether the config is free of conflicts.
    pub fn is_ok(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// The database to validate against for `family`. Empty or `"STM32F446RE"`
/// resolves to the F446RE; `"STM32F411RE"` to the F411RE. Any other value is an
/// [`UnknownFamily`] error so the CLI/LSP can warn and fall back.
pub fn database_for(family: &str) -> Result<Database, UnknownFamily> {
    match family {
        "STM32F446RE" | "" => Ok(Database::f446re()),
        "STM32F411RE" => Ok(Database::f411re()),
        other => Err(UnknownFamily(other.to_string())),
    }
}

/// The clock-tree model to validate against for `family`, mirroring
/// [`database_for`]. Unknown families fall back to the F446RE so the clock check
/// degrades the same way the pin check does (never a panic).
pub fn clock_tree_for(family: &str) -> ClockTree {
    match family {
        "STM32F411RE" => ClockTree::f411re(),
        _ => ClockTree::f446re(),
    }
}

/// The DMA model to validate against for `family`, mirroring [`clock_tree_for`].
/// Unknown families fall back to the F446RE.
pub fn dma_map_for(family: &str) -> DmaMap {
    match family {
        "STM32F411RE" => DmaMap::f411re(),
        _ => DmaMap::f446re(),
    }
}

/// The IRQ/NVIC model to validate against for `family`, mirroring
/// [`dma_map_for`]. Unknown families fall back to the F446RE.
pub fn irq_map_for(family: &str) -> IrqMap {
    match family {
        "STM32F411RE" => IrqMap::f411re(),
        _ => IrqMap::f446re(),
    }
}

/// Returned when `[device].family` names an MCU the database doesn't cover yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFamily(pub String);

impl std::fmt::Display for UnknownFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported device family {:?}: Nucleus supports STM32F446RE and STM32F411RE",
            self.0
        )
    }
}

impl std::error::Error for UnknownFamily {}

/// Parse and validate `stm32.toml` text in one call.
///
/// Returns [`ParseError`] only for malformed TOML / schema violations; hardware
/// conflicts are returned *inside* the [`CheckReport`] (a valid file can still
/// describe an invalid board).
pub fn check(text: &str) -> Result<CheckReport, ParseError> {
    let config = config::parse(text)?;
    // An unknown family is itself a conflict-worthy condition, but we model it
    // as falling back to the F446 DB; the CLI surfaces the family mismatch.
    let db = database_for(&config.device.family).unwrap_or_else(|_| Database::f446re());
    let conflicts = solver::solve(&config, &db);
    Ok(CheckReport { config, conflicts })
}

/// Like [`check`], but also reports an unsupported `[device].family`.
pub fn check_family(text: &str) -> Result<(CheckReport, Option<UnknownFamily>), ParseError> {
    let config = config::parse(text)?;
    let family_warning = database_for(&config.device.family).err();
    let db = database_for(&config.device.family).unwrap_or_else(|_| Database::f446re());
    let conflicts = solver::solve(&config, &db);
    Ok((CheckReport { config, conflicts }, family_warning))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_reports_ok_for_clean_config() {
        let report = check(
            r#"
[device]
family = "STM32F446RE"

[peripherals.usart2]
tx = "PA2"
rx = "PA3"
"#,
        )
        .unwrap();
        assert!(report.is_ok());
    }

    #[test]
    fn check_surfaces_conflicts() {
        let report = check(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"

[peripherals.tim2]
channel1 = "PA5"
"#,
        )
        .unwrap();
        assert!(!report.is_ok());
    }

    #[test]
    fn unknown_family_is_flagged() {
        let (_report, warning) = check_family(
            r#"
[device]
family = "STM32H750"
"#,
        )
        .unwrap();
        assert_eq!(warning, Some(UnknownFamily("STM32H750".to_string())));
    }

    #[test]
    fn malformed_toml_is_a_parse_error() {
        assert!(check("this is not toml = = =").is_err());
    }

    #[test]
    fn database_for_resolves_known_families() {
        assert!(database_for("STM32F446RE").is_ok());
        assert!(database_for("STM32F411RE").is_ok());
        assert!(database_for("").is_ok()); // empty falls back to F446RE
        assert!(database_for("STM32H750").is_err());
    }

    #[test]
    fn check_family_resolves_db_for_f411re() {
        // UART4 is absent on the F411RE; check_family must validate against the
        // F411 DB (not the F446 fallback) and report it, with no family warning
        // since STM32F411RE is a recognized family.
        let (report, warning) = check_family(
            "[device]\nfamily = \"STM32F411RE\"\n\n[peripherals.uart4]\ntx = \"PA0\"\nrx = \"PA1\"\n",
        )
        .unwrap();
        assert_eq!(warning, None);
        assert!(
            report.conflicts.iter().any(|c| matches!(
                c,
                Conflict::PeripheralUnavailable { peripheral, family }
                    if peripheral == "UART4" && family == "STM32F411RE"
            )),
            "got {:?}",
            report.conflicts
        );
    }
}
