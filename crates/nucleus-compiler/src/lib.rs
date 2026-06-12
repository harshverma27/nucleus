//! The Nucleus pinmux compiler.
//!
//! Owns the `stm32.toml` → diagnostics pipeline: [`config`] parses the file,
//! [`solver`] validates it against the [`nucleus_db`] constraint database, and
//! [`check`] is the one-call entry point the CLI and (later) the LSP build on.
//!
//! Phase 2 ships the parser and the constraint solver (four conflict classes).
//! HAL code generation lands in Phase 3.

pub mod codegen;
pub mod config;
pub mod model;
pub mod solver;

use nucleus_db::Database;

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

/// The database to validate against for `family`. Phase 2 supports exactly one
/// MCU (the NUCLEO-F446RE); unknown families fall back to it with the family
/// recorded in [`UnknownFamily`] so the CLI can warn.
fn database_for(family: &str) -> Result<Database, UnknownFamily> {
    match family {
        "STM32F446RE" | "" => Ok(Database::f446re()),
        other => Err(UnknownFamily(other.to_string())),
    }
}

/// Returned when `[device].family` names an MCU the database doesn't cover yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFamily(pub String);

impl std::fmt::Display for UnknownFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported device family {:?}: Nucleus currently supports only STM32F446RE",
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
    let db = Database::f446re();
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
}
