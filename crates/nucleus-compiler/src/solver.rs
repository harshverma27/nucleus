//! The hardware constraint solver.
//!
//! Takes a parsed [`Config`] and a [`Database`] and produces a list of
//! [`Conflict`]s. Phase 2 detects exactly the four conflict classes from the
//! README roadmap:
//!
//! 1. **Pin collision** — two peripheral signals on one physical pin.
//! 2. **AF mismatch** — a pin assigned to a peripheral it doesn't connect to.
//! 3. **Missing required pin** — a peripheral declared without a required pin.
//! 4. **Clock domain disabled** — a peripheral whose bus clock is turned off.
//!
//! Per the scope rules there is no DMA-collision or full clock-tree analysis.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use nucleus_db::{Database, Pin};

use crate::config::Config;
use crate::model::{self, Bus};

/// The severity of a [`Conflict`]. Every variant predating M3 is implicitly
/// [`Severity::Error`] (it makes the config un-buildable); M3's
/// [`Conflict::IrqConflict`] is the first variant to carry an explicit,
/// per-instance severity (e.g. a priority inversion may only warrant a
/// warning while an unhandled-but-enabled IRQ is fatal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A single resolved conflict. Most variants are errors (they make the config
/// un-buildable); `nucleus check` exits non-zero if any [`Severity::Error`]
/// conflicts are present. See [`Conflict::severity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// Two signals assigned to the same physical pin.
    PinCollision {
        pin: Pin,
        /// The colliding `(peripheral, signal)` users of the pin, sorted.
        users: Vec<SignalRef>,
    },
    /// A pin that does not expose the requested peripheral signal on this MCU.
    AfMismatch {
        pin: Pin,
        peripheral: String,
        signal: String,
    },
    /// A pin role whose string value is not a valid pin name.
    InvalidPin {
        peripheral: String,
        key: String,
        value: String,
    },
    /// A required pin role left unset.
    MissingPin {
        peripheral: String,
        key: String,
        signal: String,
    },
    /// A peripheral configured while its bus clock domain is disabled.
    ClockDomainDisabled { peripheral: String, bus: Bus },
    /// A peripheral instance that does not exist on the selected MCU family.
    PeripheralUnavailable { peripheral: String, family: String },
    /// A clock-tree constraint violation: an over-clocked bus, a PLL VCO/divider
    /// out of range, an invalid prescaler, or an unreachable peripheral rate.
    /// Fatal — gates codegen and the HIL runner like any other conflict.
    ClockConstraint {
        /// The offending clock-tree node (`"SYSCLK"`, `"AHB"`, `"APB1"`, `"PLL"`,
        /// or a peripheral instance name). Used for span mapping and dedup.
        node: String,
        /// Human-readable explanation; also the `Display` body.
        reason: String,
    },
    /// Two peripherals contending for the same DMA stream. Reported once per
    /// contested stream (not per pair), fatal like every conflict.
    DmaCollision {
        /// The peripheral already holding the contested stream.
        first: String,
        /// The peripheral that could not be placed.
        second: String,
        /// The contested controller, e.g. `"DMA1"`.
        controller: String,
        /// The contested stream number.
        stream: u8,
        /// An optional `(peripheral, slot label)` suggestion: a free alternative
        /// slot one of the contenders could move to.
        suggestion: Option<(String, String)>,
    },
    /// An IRQ/NVIC verification failure: an EXTI line shared by pins that are
    /// both enabled, a peripheral interrupt enabled but never handled, or an
    /// NVIC priority inversion. One flexible variant covers all three, like
    /// [`Conflict::ClockConstraint`].
    IrqConflict {
        /// The offending node: a DB peripheral name (unhandled/priority-inversion
        /// cases, for `name_to_key` lookup in the LSP) or a pin string like
        /// `"PA0"` (EXTI collision case, for a text-search fallback).
        node: String,
        /// Human-readable explanation; also the `Display` body.
        reason: String,
        /// This variant's severity; not every IRQ issue is fatal.
        severity: Severity,
    },
    /// A constraint auto-router failure: the router could not find a valid pin
    /// assignment for some peripheral role given the current constraints.
    /// Always fatal (no warning-level concept for an unroutable result).
    Unroutable {
        /// The offending node: a peripheral instance name or role, for span
        /// mapping and dedup.
        node: String,
        /// Human-readable explanation; also the `Display` body.
        reason: String,
    },
    /// A `[[test]]` block with an unparseable assertion string, or one that
    /// references a pin/peripheral the resolved family doesn't have. Always
    /// fatal (an unrunnable test can never pass).
    InvalidTest {
        /// The test's `name` field, for span mapping and dedup.
        node: String,
        /// Human-readable explanation; also the `Display` body.
        reason: String,
    },
}

impl Conflict {
    /// This conflict's severity. Every variant predating M3 is unconditionally
    /// [`Severity::Error`] (preserves current behavior exactly); only
    /// [`Conflict::IrqConflict`] carries an explicit, per-instance severity.
    pub fn severity(&self) -> Severity {
        match self {
            Conflict::IrqConflict { severity, .. } => *severity,
            _ => Severity::Error,
        }
    }
}

/// A `(peripheral, signal)` pair identifying one use of a pin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SignalRef {
    pub peripheral: String,
    pub signal: String,
}

impl fmt::Display for SignalRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.peripheral, self.signal)
    }
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Conflict::PinCollision { pin, users } => {
                let names: Vec<String> = users.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "pin collision on {pin}: assigned to {}",
                    names.join(" and ")
                )
            }
            Conflict::AfMismatch {
                pin,
                peripheral,
                signal,
            } => write!(
                f,
                "AF mismatch: {pin} has no alternate function for {peripheral}_{signal} on this MCU"
            ),
            Conflict::InvalidPin {
                peripheral,
                key,
                value,
            } => write!(
                f,
                "invalid pin: {peripheral}.{key} = {value:?} is not a valid pin name"
            ),
            Conflict::MissingPin {
                peripheral,
                key,
                signal,
            } => write!(
                f,
                "missing required pin: {peripheral} needs a {key} pin ({peripheral}_{signal})"
            ),
            Conflict::ClockDomainDisabled { peripheral, bus } => write!(
                f,
                "clock domain disabled: {peripheral} is on {} but [clocks].{} = false",
                bus.name(),
                bus.name().to_ascii_lowercase()
            ),
            Conflict::PeripheralUnavailable { peripheral, family } => {
                write!(f, "peripheral {peripheral} is not available on {family}")
            }
            Conflict::ClockConstraint { node, reason } => {
                write!(f, "clock constraint [{node}]: {reason}")
            }
            Conflict::DmaCollision {
                first,
                second,
                controller,
                stream,
                suggestion,
            } => {
                write!(
                    f,
                    "DMA collision: {first} and {second} both need {controller} stream {stream}"
                )?;
                if let Some((peripheral, slot)) = suggestion {
                    write!(f, " (move {peripheral} to {slot})")?;
                }
                Ok(())
            }
            Conflict::IrqConflict { node, reason, .. } => {
                write!(f, "IRQ conflict [{node}]: {reason}")
            }
            Conflict::Unroutable { node, reason } => {
                write!(f, "unroutable [{node}]: {reason}")
            }
            Conflict::InvalidTest { node, reason } => {
                write!(f, "invalid test [{node}]: {reason}")
            }
        }
    }
}

/// Run the solver over `config` against `db`, returning all conflicts in a
/// deterministic order (so output and tests are stable across runs).
pub fn solve(config: &Config, db: &Database) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    // pin -> the signals assigned to it, for collision detection.
    let mut pin_users: BTreeMap<Pin, Vec<SignalRef>> = BTreeMap::new();

    // BTreeMap iteration is lexical, giving deterministic ordering.
    for (instance, table) in &config.peripherals {
        let Some(roles) = model::roles_for(instance) else {
            // Unmodelled peripheral kind: nothing to check.
            continue;
        };
        let peripheral = model::peripheral_name(instance);

        // A peripheral the selected family doesn't have at all: report once and
        // skip pin/clock checks (which would otherwise emit confusing AF/missing
        // errors for a nonexistent block).
        if !db.has_peripheral(&peripheral) {
            conflicts.push(Conflict::PeripheralUnavailable {
                peripheral: peripheral.clone(),
                family: config.device.family.clone(),
            });
            continue;
        }

        // Clock-domain check: one diagnostic per peripheral, before pin work.
        if let Some(bus) = model::peripheral_bus(&peripheral) {
            let enabled = match bus {
                Bus::Ahb1 => config.clocks.ahb1,
                Bus::Apb1 => config.clocks.apb1,
                Bus::Apb2 => config.clocks.apb2,
            };
            if !enabled {
                conflicts.push(Conflict::ClockDomainDisabled {
                    peripheral: peripheral.clone(),
                    bus,
                });
            }
        }

        for role in roles {
            match table.pin_str(role.key) {
                None => {
                    if role.required {
                        conflicts.push(Conflict::MissingPin {
                            peripheral: peripheral.clone(),
                            key: role.key.to_string(),
                            signal: role.signal.to_string(),
                        });
                    }
                }
                Some(value) => {
                    let Ok(pin) = Pin::from_str(value) else {
                        conflicts.push(Conflict::InvalidPin {
                            peripheral: peripheral.clone(),
                            key: role.key.to_string(),
                            value: value.to_string(),
                        });
                        continue;
                    };
                    // AF mismatch: does this pin actually expose this signal?
                    if db.find_af(pin, &peripheral, role.signal).is_none() {
                        conflicts.push(Conflict::AfMismatch {
                            pin,
                            peripheral: peripheral.clone(),
                            signal: role.signal.to_string(),
                        });
                    }
                    // Record for collision detection regardless of AF validity:
                    // two peripherals fighting over a pin is worth reporting even
                    // if one of them is also mis-wired.
                    pin_users.entry(pin).or_default().push(SignalRef {
                        peripheral: peripheral.clone(),
                        signal: role.signal.to_string(),
                    });
                }
            }
        }
    }

    // One PinCollision per over-subscribed pin (not per pair), so a doubly-used
    // pin yields exactly one error.
    for (pin, mut users) in pin_users {
        if users.len() > 1 {
            users.sort();
            conflicts.push(Conflict::PinCollision { pin, users });
        }
    }

    // Clock-tree validation (M1) runs after the pin/AF/domain conflicts in the
    // deterministic order.
    let tree = crate::clock_tree_for(&config.device.family);
    conflicts.extend(crate::clocks::validate(config, &tree));

    // DMA arbitration (M2) runs after the clock-tree validation.
    let dma_map = crate::dma_map_for(&config.device.family);
    conflicts.extend(crate::dma::validate(config, &dma_map));

    // IRQ/NVIC verification (M3) runs last.
    let irq_map = crate::irq_map_for(&config.device.family);
    conflicts.extend(crate::irq::validate(config, &irq_map));

    // Declarative test validation (M6) depends on nothing else and nothing
    // else depends on it, so it runs last in the deterministic order.
    // `config.test` is a `Vec`, not a `BTreeMap` — iterate in document order.
    for test in &config.test {
        let is_scripted = test.kind.as_deref() == Some("scripted");
        let kind_valid = match test.kind.as_deref() {
            None | Some("declarative") | Some("scripted") => true,
            Some(other) => {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: format!("type must be \"declarative\" or \"scripted\", got {other:?}"),
                });
                false
            }
        };
        if !kind_valid {
            continue;
        }

        if is_scripted {
            if test.script.as_deref().unwrap_or("").is_empty() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "scripted test requires a non-empty `script`".to_string(),
                });
            }
            if !test.assertion.is_empty() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "scripted test must not set `assertion`".to_string(),
                });
            }
        } else {
            // declarative: existing M6 validation (assertion parse + subject check)
            if test.script.is_some() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "declarative test must not set `script`".to_string(),
                });
                continue;
            }
            if test.assertion.is_empty() {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: "declarative test requires an `assertion`".to_string(),
                });
                continue;
            }

            let assertion = match crate::assertion::parse(&test.assertion) {
                Ok(assertion) => assertion,
                Err(reason) => {
                    conflicts.push(Conflict::InvalidTest {
                        node: test.name.clone(),
                        reason,
                    });
                    continue;
                }
            };

            let subject_invalid = match &assertion {
                crate::assertion::Assertion::PinToggles { pin, .. }
                | crate::assertion::Assertion::PinState { pin, .. } => {
                    if Pin::from_str(pin).is_err() {
                        Some(format!("'{pin}' is not a valid pin name"))
                    } else {
                        None
                    }
                }
                crate::assertion::Assertion::UartEcho { instance, .. } => {
                    let peripheral = model::peripheral_name(instance);
                    if !db.has_peripheral(&peripheral) {
                        Some(format!(
                            "peripheral {instance} is not available on this family"
                        ))
                    } else {
                        None
                    }
                }
                crate::assertion::Assertion::ItmEvent { .. } => None,
            };

            if let Some(reason) = subject_invalid {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason,
                });
                continue;
            }
        }

        // shared backend validation applies to both declarative and scripted.
        if let Some(backend) = &test.backend {
            if backend != "qemu" && backend != "hardware" && backend != "both" {
                conflicts.push(Conflict::InvalidTest {
                    node: test.name.clone(),
                    reason: format!(
                        "backend must be \"qemu\", \"hardware\", or \"both\", got {backend:?}"
                    ),
                });
            }
        }
    }

    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn db() -> Database {
        Database::f446re()
    }

    fn solve_toml(text: &str) -> Vec<Conflict> {
        let cfg = config::parse(text).unwrap();
        solve(&cfg, &db())
    }

    #[test]
    fn clean_config_has_no_conflicts() {
        let conflicts = solve_toml(
            r#"
[peripherals.usart2]
tx = "PA2"
rx = "PA3"

[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
nss = "PA4"

[peripherals.i2c1]
sda = "PB9"
scl = "PB8"
"#,
        );
        assert_eq!(
            conflicts,
            vec![],
            "expected clean config, got {conflicts:?}"
        );
    }

    #[test]
    fn detects_pin_collision() {
        // PA5 is SPI1_SCK and also (wrongly) USART2... we put two real signals
        // on PA5 to force a collision.
        let conflicts = solve_toml(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"

[peripherals.tim2]
channel1 = "PA5"
"#,
        );
        let collisions: Vec<_> = conflicts
            .iter()
            .filter(|c| matches!(c, Conflict::PinCollision { .. }))
            .collect();
        assert_eq!(collisions.len(), 1, "got {conflicts:?}");
        if let Conflict::PinCollision { pin, users } = collisions[0] {
            assert_eq!(pin.to_string(), "PA5");
            assert_eq!(users.len(), 2);
        }
    }

    #[test]
    fn detects_af_mismatch() {
        // PB0 does not carry USART2_TX on the F446.
        let conflicts = solve_toml(
            r#"
[peripherals.usart2]
tx = "PB0"
rx = "PA3"
"#,
        );
        assert!(
            conflicts.iter().any(|c| matches!(
                c,
                Conflict::AfMismatch { pin, signal, .. }
                    if pin.to_string() == "PB0" && signal == "TX"
            )),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn detects_missing_required_pin() {
        // SPI1 without MOSI.
        let conflicts = solve_toml(
            r#"
[peripherals.spi1]
miso = "PA6"
sck = "PA5"
"#,
        );
        assert!(
            conflicts.iter().any(|c| matches!(
                c,
                Conflict::MissingPin { peripheral, signal, .. }
                    if peripheral == "SPI1" && signal == "MOSI"
            )),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn missing_optional_pin_is_not_a_conflict() {
        // SPI1 without NSS (optional) is fine.
        let conflicts = solve_toml(
            r#"
[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
"#,
        );
        assert_eq!(conflicts, vec![]);
    }

    #[test]
    fn detects_clock_domain_disabled() {
        // SPI1 lives on APB2; disabling APB2 must flag it.
        let conflicts = solve_toml(
            r#"
[clocks]
apb2 = false

[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
"#,
        );
        assert!(
            conflicts.iter().any(|c| matches!(
                c,
                Conflict::ClockDomainDisabled { peripheral, bus }
                    if peripheral == "SPI1" && *bus == Bus::Apb2
            )),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn detects_peripheral_unavailable_on_family() {
        // UART4 exists on the F446 but not on the F411RE package. Configured
        // under family = STM32F411RE it must produce exactly one
        // PeripheralUnavailable conflict and no spurious pin conflicts.
        let cfg = config::parse(
            "[device]\nfamily = \"STM32F411RE\"\n\n[peripherals.uart4]\ntx = \"PA0\"\nrx = \"PA1\"\n",
        )
        .unwrap();
        let conflicts = solve(&cfg, &Database::f411re());
        assert_eq!(
            conflicts,
            vec![Conflict::PeripheralUnavailable {
                peripheral: "UART4".to_string(),
                family: "STM32F411RE".to_string(),
            }],
            "got {conflicts:?}"
        );
    }

    #[test]
    fn invalid_pin_name_reported() {
        let conflicts = solve_toml(
            r#"
[peripherals.usart2]
tx = "PZ9"
rx = "PA3"
"#,
        );
        assert!(
            conflicts
                .iter()
                .any(|c| matches!(c, Conflict::InvalidPin { value, .. } if value == "PZ9")),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn pre_m3_conflict_variants_are_all_severity_error() {
        // Every conflict variant that existed before M3 is implicitly an
        // error; introducing `Severity` (and the new `IrqConflict` variant,
        // which carries its own explicit severity) must not change that.
        let pin = Pin::from_str("PA5").unwrap();
        assert_eq!(
            Conflict::PinCollision { pin, users: vec![] }.severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::AfMismatch {
                pin,
                peripheral: "USART2".to_string(),
                signal: "TX".to_string(),
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::InvalidPin {
                peripheral: "USART2".to_string(),
                key: "tx".to_string(),
                value: "PZ9".to_string(),
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::MissingPin {
                peripheral: "SPI1".to_string(),
                key: "mosi".to_string(),
                signal: "MOSI".to_string(),
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::ClockDomainDisabled {
                peripheral: "SPI1".to_string(),
                bus: Bus::Apb2,
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::PeripheralUnavailable {
                peripheral: "UART4".to_string(),
                family: "STM32F411RE".to_string(),
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::ClockConstraint {
                node: "SYSCLK".to_string(),
                reason: "over-clocked".to_string(),
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::DmaCollision {
                first: "USART2".to_string(),
                second: "SPI1".to_string(),
                controller: "DMA1".to_string(),
                stream: 3,
                suggestion: None,
            }
            .severity(),
            Severity::Error
        );
    }

    #[test]
    fn irq_conflict_severity_is_per_instance() {
        // Unlike every other variant, `IrqConflict`'s severity is whatever the
        // caller set, not hardcoded.
        assert_eq!(
            Conflict::IrqConflict {
                node: "EXTI0".to_string(),
                reason: "enabled but unhandled".to_string(),
                severity: Severity::Error,
            }
            .severity(),
            Severity::Error
        );
        assert_eq!(
            Conflict::IrqConflict {
                node: "PA0".to_string(),
                reason: "shares EXTI0 with PB0".to_string(),
                severity: Severity::Warning,
            }
            .severity(),
            Severity::Warning
        );
    }

    #[test]
    fn unroutable_severity_is_always_error() {
        // `Unroutable` is always a fatal error; no per-instance severity.
        assert_eq!(
            Conflict::Unroutable {
                node: "USART2_TX".to_string(),
                reason: "no free pins satisfy constraints".to_string(),
            }
            .severity(),
            Severity::Error
        );
    }

    #[test]
    fn unroutable_display_format() {
        // `Unroutable`'s Display output must include both node and reason,
        // following the `IrqConflict` pattern.
        let conflict = Conflict::Unroutable {
            node: "SPI1_MOSI".to_string(),
            reason: "all candidate pins are occupied".to_string(),
        };
        let display = format!("{}", conflict);
        assert!(display.contains("unroutable"), "display: {display}");
        assert!(display.contains("SPI1_MOSI"), "display: {display}");
        assert!(
            display.contains("all candidate pins are occupied"),
            "display: {display}"
        );
    }

    #[test]
    fn invalid_test_display_format() {
        // `InvalidTest`'s Display output must include both node and reason.
        let conflict = Conflict::InvalidTest {
            node: "uart2_echo".to_string(),
            reason: "'NOTAPIN' is not a valid pin name".to_string(),
        };
        let display = format!("{}", conflict);
        assert!(display.contains("invalid test"), "display: {display}");
        assert!(display.contains("uart2_echo"), "display: {display}");
        assert!(
            display.contains("'NOTAPIN' is not a valid pin name"),
            "display: {display}"
        );
    }

    #[test]
    fn valid_test_referencing_real_pin_has_no_conflict() {
        let conflicts = solve_toml(
            r#"
[[test]]
name = "blink_check"
assertion = "pin PA5 toggles at 1Hz ±5%"
"#,
        );
        assert!(
            !conflicts
                .iter()
                .any(|c| matches!(c, Conflict::InvalidTest { .. })),
            "{conflicts:?}"
        );
    }

    #[test]
    fn unparseable_assertion_yields_invalid_test() {
        let conflicts = solve_toml(
            r#"
[[test]]
name = "garbage_test"
assertion = "this is not an assertion at all"
"#,
        );
        let invalid: Vec<&Conflict> = conflicts
            .iter()
            .filter(|c| matches!(c, Conflict::InvalidTest { .. }))
            .collect();
        assert_eq!(invalid.len(), 1, "{conflicts:?}");
        let display = format!("{}", invalid[0]);
        assert!(display.contains("garbage_test"), "display: {display}");
    }

    #[test]
    fn pin_assertion_with_invalid_pin_name_yields_invalid_test() {
        let conflicts = solve_toml(
            r#"
[[test]]
name = "bad_pin_test"
assertion = "pin NOTAPIN toggles at 1Hz ±5%"
"#,
        );
        let invalid: Vec<&Conflict> = conflicts
            .iter()
            .filter(|c| matches!(c, Conflict::InvalidTest { .. }))
            .collect();
        assert_eq!(invalid.len(), 1, "{conflicts:?}");
    }

    #[test]
    fn uart_echo_with_unavailable_peripheral_yields_invalid_test() {
        let conflicts = solve_toml(
            r#"
[[test]]
name = "fake_periph_test"
assertion = "FAKEPERIPH echoes \"x\" within 10ms"
"#,
        );
        let invalid: Vec<&Conflict> = conflicts
            .iter()
            .filter(|c| matches!(c, Conflict::InvalidTest { .. }))
            .collect();
        assert_eq!(invalid.len(), 1, "{conflicts:?}");
        let display = format!("{}", invalid[0]);
        assert!(display.contains("FAKEPERIPH"), "display: {display}");
    }

    #[test]
    fn invalid_backend_value_yields_invalid_test() {
        let conflicts = solve_toml(
            r#"
[[test]]
name = "bad_backend_test"
assertion = "pin PA5 is high within 10ms"
backend = "emulator"
"#,
        );
        let invalid: Vec<&Conflict> = conflicts
            .iter()
            .filter(|c| matches!(c, Conflict::InvalidTest { .. }))
            .collect();
        assert_eq!(invalid.len(), 1, "{conflicts:?}");
        let display = format!("{}", invalid[0]);
        assert!(display.contains("backend"), "display: {display}");
        assert!(display.contains("emulator"), "display: {display}");
    }

    #[test]
    fn one_valid_one_invalid_test_yields_exactly_one_invalid_test_conflict() {
        let conflicts = solve_toml(
            r#"
[[test]]
name = "good_test"
assertion = "pin PA5 is high within 10ms"

[[test]]
name = "bad_test"
assertion = "not a real assertion"
"#,
        );
        let invalid: Vec<&Conflict> = conflicts
            .iter()
            .filter(|c| matches!(c, Conflict::InvalidTest { .. }))
            .collect();
        assert_eq!(invalid.len(), 1, "{conflicts:?}");
        let display = format!("{}", invalid[0]);
        assert!(display.contains("bad_test"), "display: {display}");
    }
}
