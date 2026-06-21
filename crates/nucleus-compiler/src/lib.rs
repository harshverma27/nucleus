//! The Nucleus pinmux compiler.
//!
//! Owns the `stm32.toml` → diagnostics pipeline: [`config`] parses the file,
//! [`solver`] validates it against the [`nucleus_db`] constraint database, and
//! [`check`]/[`check_family`]/[`route_family`]/[`test_plan`] are the one-call
//! entry points the CLI and LSP build on.
//!
//! The solver has grown from v1's four conflict classes to eleven (clock-tree,
//! DMA, IRQ/NVIC, auto-router, and `[[test]]` validation across v2's M1–M6).
//! HAL code generation ([`codegen`]) shipped in v1 Phase 3.

pub mod assertion;
pub mod clocks;
pub mod codegen;
pub mod config;
pub mod dma;
pub mod irq;
pub mod model;
pub mod router;
pub mod solver;

use nucleus_db::clock::ClockTree;
use nucleus_db::dma::DmaMap;
use nucleus_db::irq::IrqMap;
use nucleus_db::Database;

pub use assertion::Assertion;
pub use clocks::{PeripheralFreq, ResolvedClocks};
pub use codegen::{generate, Generated};
pub use config::{Config, ParseError};
pub use solver::{Conflict, Severity};

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
    ///
    /// A config is considered OK if it contains no [`Severity::Error`] conflicts;
    /// warnings do not make it fail.
    pub fn is_ok(&self) -> bool {
        !self
            .conflicts
            .iter()
            .any(|c| c.severity() == Severity::Error)
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

/// The outcome of routing one `stm32.toml`: the parsed config, and either the
/// rendered, fully-pinned TOML text (success) or the conflicts that made
/// routing fail.
#[derive(Debug, Clone)]
pub struct RouteReport {
    /// The parsed config that was routed.
    pub config: Config,
    /// `Ok(rendered_toml)` on a successful route — the original `text`, with
    /// every newly-routed pin spliced in, comments/formatting/ordering
    /// elsewhere untouched. `Err(conflicts)` on failure.
    pub outcome: Result<String, Vec<Conflict>>,
}

/// Like [`check_family`], but runs the M4 auto-router ([`router::route`])
/// instead of just validating, and renders a successful route back into TOML
/// text via `toml_edit` (preserving the original file's comments and
/// formatting, since only the newly-solved pins are spliced in).
pub fn route_family(text: &str) -> Result<(RouteReport, Option<UnknownFamily>), ParseError> {
    let config = config::parse(text)?;
    let family_warning = database_for(&config.device.family).err();
    let db = database_for(&config.device.family).unwrap_or_else(|_| Database::f446re());

    let outcome = match router::route(&config, &db) {
        Ok(assignment) => {
            // `text` parsed cleanly as our own TOML schema above, so it must
            // also parse as a generic `toml_edit` document; an empty
            // assignment leaves `doc` (and thus the rendered string)
            // unchanged.
            let mut doc = text
                .parse::<toml_edit::DocumentMut>()
                .expect("text already parsed as Config, so it must parse as toml_edit too");
            for ((instance, key), pin) in &assignment {
                doc["peripherals"][instance][key] = toml_edit::value(pin.to_string());
            }
            Ok(doc.to_string())
        }
        Err(conflicts) => Err(conflicts),
    };

    Ok((RouteReport { config, outcome }, family_warning))
}

/// Which backend(s) a [`CompiledTest`] should run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSelect {
    Qemu,
    Hardware,
    Both,
}

/// A compiled test's payload: a declarative assertion (M6) or a scripted
/// cargo-test pointer (M7).
#[derive(Debug, Clone, PartialEq)]
pub enum TestBody {
    Declarative(Assertion),
    Scripted { script: String },
}

/// One `[[test]]` block, parsed and validated: ready for `nucleus-hil` to
/// execute. Never carries raw TOML or [`Config`] — [`test_plan`] is the only
/// place a [`Conflict::InvalidTest`] can still occur; once you have a
/// `CompiledTest` it is guaranteed runnable (the assertion parsed, its pin/
/// peripheral reference exists on the resolved family, its backend value is
/// one of the three recognized strings).
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTest {
    pub name: String,
    pub body: TestBody,
    pub timeout: std::time::Duration,
    pub backend: BackendSelect,
}

impl CompiledTest {
    /// The declarative assertion, if this is a declarative test.
    pub fn assertion(&self) -> Option<&Assertion> {
        match &self.body {
            TestBody::Declarative(a) => Some(a),
            TestBody::Scripted { .. } => None,
        }
    }
}

/// Parse and validate every `[[test]]` block in `text`, returning the
/// runnable plan on success or the conflicts ([`Conflict::InvalidTest`] plus
/// any other conflict the config has — an unrunnable peripheral wiring is
/// just as fatal to a test as a bad assertion) that block it.
///
/// Reuses [`solver::solve`]'s validation rather than re-validating: a
/// [`CompiledTest`] is only ever produced when the *entire* config — not
/// just its `[[test]]` blocks — has zero [`Severity::Error`] conflicts, so a
/// test plan can never be handed to `nucleus-hil` for a config that
/// wouldn't even build.
pub fn test_plan(text: &str) -> Result<Result<Vec<CompiledTest>, Vec<Conflict>>, ParseError> {
    let config = config::parse(text)?;
    let db = database_for(&config.device.family).unwrap_or_else(|_| Database::f446re());
    let conflicts = solver::solve(&config, &db);

    if conflicts.iter().any(|c| c.severity() == Severity::Error) {
        return Ok(Err(conflicts));
    }

    let compiled = config
        .test
        .iter()
        .map(|t| {
            let backend = match t.backend.as_deref() {
                None | Some("both") => BackendSelect::Both,
                Some("qemu") => BackendSelect::Qemu,
                Some("hardware") => BackendSelect::Hardware,
                Some(other) => unreachable!(
                    "solve() already validated config.test backend values; \
                     {other:?} would have produced an InvalidTest conflict above"
                ),
            };
            let body = if t.kind.as_deref() == Some("scripted") {
                TestBody::Scripted {
                    script: t
                        .script
                        .clone()
                        .expect("solve() validated scripted script present"),
                }
            } else {
                TestBody::Declarative(assertion::parse(&t.assertion).expect(
                    "solve() already validated every config.test entry's assertion string; \
                     a parse failure here would have produced an InvalidTest conflict above",
                ))
            };
            CompiledTest {
                name: t.name.clone(),
                body,
                timeout: std::time::Duration::from_millis(t.timeout_ms),
                backend,
            }
        })
        .collect();

    Ok(Ok(compiled))
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

    #[test]
    fn is_ok_accepts_warnings_only() {
        // A config with only warning-level conflicts (priority inversion) should
        // pass is_ok(). Priority inversion occurs when dma_priority > irq_priority.
        let report = check(
            r#"
[device]
family = "STM32F446RE"

[peripherals.usart2]
tx = "PA2"
rx = "PA3"
irq = true
irq_priority = 2
dma = true
dma_priority = 5
"#,
        )
        .unwrap();
        // Should have exactly one warning (priority inversion)
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].severity(), Severity::Warning);
        // is_ok() should return true because there are no errors
        assert!(report.is_ok());
    }

    #[test]
    fn is_ok_rejects_errors() {
        // A config with any error-level conflict should fail is_ok().
        // Use the existing pin collision test case which produces an error.
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
        // Should have at least one error (pin collision)
        assert!(!report.conflicts.is_empty());
        assert!(report
            .conflicts
            .iter()
            .any(|c| c.severity() == Severity::Error));
        // is_ok() should return false because there are errors
        assert!(!report.is_ok());
    }

    // --- route_family --------------------------------------------------

    #[test]
    fn route_family_renders_routed_pins_and_preserves_comments() {
        let text = r#"# top-of-file comment, must survive
[device]
family = "STM32F446RE"

# usart2 comment, must survive
[peripherals.usart2]
"#;
        let (report, warning) = route_family(text).unwrap();
        assert_eq!(warning, None);
        let rendered = report.outcome.expect("should route");

        // The original comments/structure are untouched.
        assert!(rendered.contains("# top-of-file comment, must survive"));
        assert!(rendered.contains("# usart2 comment, must survive"));
        assert!(rendered.contains("[device]"));
        assert!(rendered.contains("family = \"STM32F446RE\""));

        // The newly-routed pins are spliced in.
        assert!(
            rendered.contains("tx = \"PA2\""),
            "got rendered:\n{rendered}"
        );
        assert!(
            rendered.contains("rx = \"PA3\""),
            "got rendered:\n{rendered}"
        );

        // And the rendered text re-parses to a fully-pinned, clean config.
        let reparsed = check(&rendered).unwrap();
        assert!(reparsed.is_ok(), "got {:?}", reparsed.conflicts);
    }

    #[test]
    fn route_family_reports_conflicts_on_unroutable_config() {
        // USART2_TX's only candidate is PA2; pre-occupy it with an unrelated
        // peripheral signal so routing usart2 fails.
        let text = r#"[peripherals.tim5]
channel3 = "PA2"

[peripherals.usart2]
"#;
        let (report, warning) = route_family(text).unwrap();
        assert_eq!(warning, None);
        let conflicts = report.outcome.expect_err("should be unroutable");
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(&conflicts[0], Conflict::Unroutable { .. }));
    }

    #[test]
    fn route_family_round_trips_an_already_fully_pinned_config() {
        let text = "[peripherals.usart2]\ntx = \"PA2\"\nrx = \"PA3\"\n";
        let (report, _warning) = route_family(text).unwrap();
        let rendered = report.outcome.expect("should route (no-op)");

        // Nothing needed routing, so the rendered text is byte-identical to
        // the input (re-parsing the same document and never mutating it).
        assert_eq!(rendered, text);
    }

    #[test]
    fn route_family_flags_unknown_family_but_still_routes() {
        // Mirrors `unknown_family_is_flagged`: an unsupported family produces
        // a warning, not a hard failure, and routing still proceeds against
        // the F446RE fallback DB (same discipline as `check_family`).
        let text = r#"[device]
family = "STM32H750"

[peripherals.usart2]
"#;
        let (report, warning) = route_family(text).unwrap();
        assert_eq!(warning, Some(UnknownFamily("STM32H750".to_string())));
        let rendered = report
            .outcome
            .expect("should route against the fallback DB");
        assert!(rendered.contains("tx = \"PA2\""));
        assert!(rendered.contains("rx = \"PA3\""));
    }

    #[test]
    fn route_family_malformed_toml_is_a_parse_error() {
        assert!(route_family("this is not toml = = =").is_err());
    }

    // --- test_plan ------------------------------------------------------

    #[test]
    fn test_plan_returns_one_compiled_test_for_a_valid_block() {
        let plan = test_plan(
            r#"
[device]
family = "STM32F446RE"

[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "t1");
        assert!(matches!(
            plan[0].assertion(),
            Some(Assertion::PinState { pin, level: true, within })
                if pin == "PA5" && *within == std::time::Duration::from_millis(10)
        ));
    }

    #[test]
    fn test_plan_reports_invalid_test_for_garbage_assertion() {
        let conflicts = test_plan(
            r#"
[[test]]
name = "bad"
assertion = "this is not an assertion at all"
"#,
        )
        .unwrap()
        .unwrap_err();
        assert!(
            conflicts
                .iter()
                .any(|c| matches!(c, Conflict::InvalidTest { .. })),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn test_plan_falls_back_to_f446_db_for_unknown_family() {
        let plan = test_plan(
            r#"
[device]
family = "STM32H750"

[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn test_plan_maps_backend_none_to_both() {
        let plan = test_plan(
            r#"
[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan[0].backend, BackendSelect::Both);
    }

    #[test]
    fn test_plan_maps_backend_qemu() {
        let plan = test_plan(
            r#"
[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
backend = "qemu"
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan[0].backend, BackendSelect::Qemu);
    }

    #[test]
    fn test_plan_maps_backend_hardware() {
        let plan = test_plan(
            r#"
[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
backend = "hardware"
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan[0].backend, BackendSelect::Hardware);
    }

    #[test]
    fn test_plan_maps_backend_both_explicit() {
        let plan = test_plan(
            r#"
[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
backend = "both"
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan[0].backend, BackendSelect::Both);
    }

    #[test]
    fn test_plan_converts_timeout_ms_to_duration() {
        let plan = test_plan(
            r#"
[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
timeout_ms = 250
"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan[0].timeout, std::time::Duration::from_millis(250));
    }

    #[test]
    fn test_plan_fails_on_unrelated_conflict_even_with_valid_test() {
        let conflicts = test_plan(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"

[peripherals.tim2]
channel1 = "PA5"

[[test]]
name = "t1"
assertion = "pin PA5 is high within 10ms"
"#,
        )
        .unwrap()
        .unwrap_err();
        assert!(!conflicts
            .iter()
            .any(|c| matches!(c, Conflict::InvalidTest { .. })));
        assert!(conflicts.iter().any(|c| c.severity() == Severity::Error));
    }

    #[test]
    fn scripted_test_compiles_to_scripted_body() {
        let toml = r#"
[[test]]
name = "uart_loopback"
type = "scripted"
script = "uart_loopback"
backend = "both"
"#;
        let plan = test_plan(toml).unwrap().unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "uart_loopback");
        assert!(matches!(
            &plan[0].body,
            crate::TestBody::Scripted { script } if script == "uart_loopback"
        ));
    }

    #[test]
    fn scripted_test_without_script_is_invalid() {
        let toml = r#"
[[test]]
name = "bad"
type = "scripted"
"#;
        let conflicts = test_plan(toml).unwrap().unwrap_err();
        assert!(conflicts
            .iter()
            .any(|c| matches!(c, Conflict::InvalidTest { .. })));
    }

    #[test]
    fn declarative_test_with_script_is_invalid() {
        let toml = r#"
[[test]]
name = "bad"
assertion = "pin PA5 is high within 10ms"
script = "nope"
"#;
        let conflicts = test_plan(toml).unwrap().unwrap_err();
        assert!(conflicts
            .iter()
            .any(|c| matches!(c, Conflict::InvalidTest { .. })));
    }
}
