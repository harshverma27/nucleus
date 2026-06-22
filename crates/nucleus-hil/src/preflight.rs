//! The one check every [`crate::Backend::start`] must run before touching the
//! emulator or hardware: never execute a config the verifier rejected.

use nucleus_compiler::{CheckReport, Severity};

use crate::backend::HilError;

/// Reject `report` if it carries any [`Severity::Error`] conflict. Warnings
/// don't block a run, mirroring [`CheckReport::is_ok`].
pub fn gate(report: &CheckReport) -> Result<(), HilError> {
    if report.is_ok() {
        Ok(())
    } else {
        let errors = report
            .conflicts
            .iter()
            .filter(|c| c.severity() == Severity::Error)
            .cloned()
            .collect();
        Err(HilError::Preflight(errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nucleus_compiler::check;

    #[test]
    fn clean_config_passes() {
        let report = check(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
"#,
        )
        .expect("valid toml");
        assert!(gate(&report).is_ok());
    }

    #[test]
    fn conflicting_config_is_rejected() {
        // Two real signals forced onto PA5 — same fixture nucleus-compiler's
        // own solver tests use to trigger Conflict::PinCollision.
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
        .expect("valid toml");
        assert!(!report.is_ok());

        match gate(&report) {
            Err(HilError::Preflight(conflicts)) => assert!(!conflicts.is_empty()),
            other => panic!("expected Preflight rejection, got {other:?}"),
        }
    }

    #[test]
    fn empty_config_passes() {
        let report = check("").expect("empty toml parses");
        assert!(gate(&report).is_ok());
    }
}
