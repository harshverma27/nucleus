//! The IRQ/NVIC verifier (Nucleus v2 milestone M3).
//!
//! Turns the "did I wire interrupts correctly?" question into four concrete
//! checks against the family's [`nucleus_db::irq::IrqMap`]:
//!
//! 1. **Unhandled IRQ** — a peripheral table opts into `irq = true` for a
//!    peripheral the family does not model an NVIC vector for.
//! 2. **EXTI collision** — two `[[exti]]` entries land on the same EXTI line
//!    (0–15), which is shared across all eight GPIO ports, so only one pin per
//!    line can actually trigger that line's vector.
//! 3. **Priority inversion (DMA/IRQ)** — a peripheral whose DMA request is
//!    *less* urgent (numerically larger `dma_priority`) than its own IRQ
//!    (`irq_priority`), which can starve the DMA completion interrupt behind
//!    the peripheral's own ISR.
//! 4. **Priority inversion (EXTI/IRQ)** — an `[[exti]]` entry whose
//!    `priority` is *less* urgent than the `irq_priority` of a peripheral
//!    sharing the same physical pin. EXTI taps the GPIO input register
//!    regardless of AF mode, so a pin can be both a peripheral signal and an
//!    EXTI source at once; a mismatched priority can starve the edge
//!    interrupt the peripheral depends on.
//!
//! Everything here is **pure and synchronous** (config + model → conflicts),
//! mirroring [`crate::dma::validate`] and [`crate::clocks::validate`]: all
//! vector data comes from the model, never hard-coded, so the F446 and F411
//! differ automatically.
//!
//! ## Config surface
//!
//! A peripheral opts into IRQ verification explicitly, the same opt-in
//! discipline as `dma`'s `dma` key — the solver never infers it from the
//! peripheral kind:
//!
//! ```toml
//! [peripherals.usart2]
//! tx = "PA2"
//! rx = "PA3"
//! irq = true            # USART2 enables its NVIC interrupt
//! irq_priority = 5
//! dma = true
//! dma_priority = 2       # numerically smaller = more urgent
//!
//! [[exti]]
//! pin = "PA0"
//! priority = 3
//! ```

use std::collections::BTreeMap;
use std::str::FromStr;

use nucleus_db::irq::IrqMap;
use nucleus_db::Pin;

use crate::config::Config;
use crate::solver::{Conflict, Severity};

fn ic(node: String, reason: String, severity: Severity) -> Conflict {
    Conflict::IrqConflict {
        node,
        reason,
        severity,
    }
}

/// Validate the configured peripherals' IRQ/EXTI setup against the family
/// model, returning [`Conflict::IrqConflict`] entries in a fixed,
/// deterministic order. Called by the solver after the DMA arbitration.
pub fn validate(config: &Config, map: &IrqMap) -> Vec<Conflict> {
    let mut out = Vec::new();

    out.extend(unhandled_irqs(config, map));
    out.extend(exti_collisions(config));
    out.extend(priority_inversions(config));
    out.extend(exti_priority_inversions(config));

    out
}

/// **Unhandled IRQ**: a peripheral table with `irq = true` naming a
/// peripheral the family's [`IrqMap`] does not model a vector for.
fn unhandled_irqs(config: &Config, map: &IrqMap) -> Vec<Conflict> {
    let mut out = Vec::new();
    for (instance, table) in &config.peripherals {
        let Some(true) = table.0.get("irq").and_then(toml::Value::as_bool) else {
            continue;
        };
        let peripheral = crate::model::peripheral_name(instance);
        if !map.has_peripheral(&peripheral) {
            out.push(ic(
                peripheral.clone(),
                format!(
                    "{peripheral} has `irq = true` but no NVIC vector is modeled for it on this family"
                ),
                Severity::Error,
            ));
        }
    }
    out
}

/// **EXTI collision**: two `[[exti]]` entries whose pin numbers (the EXTI
/// line, shared across all eight ports) collide. Unparsable pin strings are
/// reported individually (mirroring [`Conflict::InvalidPin`]) rather than
/// silently dropped, but otherwise skipped from collision grouping.
fn exti_collisions(config: &Config) -> Vec<Conflict> {
    let mut out = Vec::new();

    // line -> the pins claiming it, in declaration order.
    let mut by_line: BTreeMap<u8, Vec<Pin>> = BTreeMap::new();
    for entry in &config.exti {
        match Pin::from_str(&entry.pin) {
            Ok(pin) => by_line.entry(pin.number).or_default().push(pin),
            Err(_) => out.push(ic(
                entry.pin.clone(),
                format!("invalid EXTI pin {:?}: not a valid pin name", entry.pin),
                Severity::Error,
            )),
        }
    }

    // One conflict per contested line (not per pair), naming every distinct
    // port claiming it — matches the PinCollision/DmaCollision dedup
    // discipline.
    for (line, pins) in by_line {
        let mut distinct_ports: Vec<Pin> = Vec::new();
        for pin in pins {
            if !distinct_ports.contains(&pin) {
                distinct_ports.push(pin);
            }
        }
        if distinct_ports.len() > 1 {
            let names: Vec<String> = distinct_ports.iter().map(ToString::to_string).collect();
            out.push(ic(
                names[0].clone(),
                format!(
                    "EXTI{line} is shared by {} but only one can trigger it",
                    names.join(" and ")
                ),
                Severity::Error,
            ));
        }
    }

    out
}

/// **Priority inversion**: a peripheral with both `dma_priority` and
/// `irq_priority` set, where the DMA priority is numerically less urgent
/// (larger) than the IRQ priority — the peripheral's own ISR would preempt
/// the DMA completion interrupt that's supposed to be servicing it.
fn priority_inversions(config: &Config) -> Vec<Conflict> {
    let mut out = Vec::new();
    for (instance, table) in &config.peripherals {
        let Some(dma_priority) = table
            .0
            .get("dma_priority")
            .and_then(toml::Value::as_integer)
        else {
            continue;
        };
        let Some(irq_priority) = table
            .0
            .get("irq_priority")
            .and_then(toml::Value::as_integer)
        else {
            continue;
        };
        if dma_priority > irq_priority {
            let peripheral = crate::model::peripheral_name(instance);
            out.push(ic(
                peripheral.clone(),
                format!(
                    "{peripheral}: DMA priority {dma_priority} is less urgent than its IRQ priority {irq_priority} (priority inversion)"
                ),
                Severity::Warning,
            ));
        }
    }
    out
}

/// **EXTI priority inversion**: an `[[exti]]` entry's `priority` that is
/// numerically less urgent (larger) than the `irq_priority` of a peripheral
/// whose pin role names the same physical pin as the EXTI entry — EXTI taps
/// the GPIO input register regardless of AF mode, so a pin can simultaneously
/// be a peripheral signal and an EXTI source. A less-urgent EXTI priority
/// means the edge interrupt the peripheral relies on can be starved behind
/// less important work.
fn exti_priority_inversions(config: &Config) -> Vec<Conflict> {
    let mut out = Vec::new();
    for entry in &config.exti {
        let Some(exti_priority) = entry.priority else {
            continue;
        };
        let Ok(exti_pin) = Pin::from_str(&entry.pin) else {
            continue;
        };
        for (instance, table) in &config.peripherals {
            let Some(true) = table.0.get("irq").and_then(toml::Value::as_bool) else {
                continue;
            };
            let Some(irq_priority) = table
                .0
                .get("irq_priority")
                .and_then(toml::Value::as_integer)
            else {
                continue;
            };
            let Some(roles) = crate::model::roles_for(instance) else {
                continue;
            };
            let shares_pin = roles.iter().any(|role| {
                table
                    .pin_str(role.key)
                    .and_then(|v| Pin::from_str(v).ok())
                    .is_some_and(|p| p == exti_pin)
            });
            if !shares_pin {
                continue;
            }
            if i64::from(exti_priority) > irq_priority {
                let peripheral = crate::model::peripheral_name(instance);
                out.push(ic(
                    peripheral.clone(),
                    format!(
                        "EXTI priority {exti_priority} on {} is less urgent than {peripheral}'s IRQ priority {irq_priority} on the same pin (priority inversion)",
                        entry.pin
                    ),
                    Severity::Warning,
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn f446() -> IrqMap {
        IrqMap::f446re()
    }
    fn f411() -> IrqMap {
        IrqMap::f411re()
    }

    fn validate_toml(text: &str, map: &IrqMap) -> Vec<Conflict> {
        let cfg = config::parse(text).unwrap();
        validate(&cfg, map)
    }

    #[test]
    fn clean_config_has_no_conflicts() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn modeled_peripheral_irq_true_is_clean() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn unmodeled_peripheral_irq_true_errors() {
        // TIM9 has no modeled NVIC vector row.
        let text = "[peripherals.tim9]\nchannel1=\"PE5\"\nirq=true\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        match &conflicts[0] {
            Conflict::IrqConflict {
                node,
                severity,
                reason,
            } => {
                assert_eq!(node, "TIM9");
                assert_eq!(*severity, Severity::Error);
                assert!(reason.contains("no NVIC vector"), "{reason}");
            }
            other => panic!("expected IrqConflict, got {other:?}"),
        }
    }

    #[test]
    fn irq_false_is_never_flagged() {
        let text = "[peripherals.tim9]\nchannel1=\"PE5\"\nirq=false\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn two_exti_pins_same_line_different_ports_collide() {
        // PA0 and PB0 both claim EXTI line 0.
        let text = "[[exti]]\npin=\"PA0\"\n\n[[exti]]\npin=\"PB0\"\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        match &conflicts[0] {
            Conflict::IrqConflict {
                node,
                reason,
                severity,
            } => {
                assert_eq!(node, "PA0");
                assert_eq!(*severity, Severity::Error);
                assert!(reason.contains("PA0") && reason.contains("PB0"), "{reason}");
                assert!(reason.contains("EXTI0"), "{reason}");
            }
            other => panic!("expected IrqConflict, got {other:?}"),
        }
    }

    #[test]
    fn exti_pins_different_lines_are_clean() {
        let text = "[[exti]]\npin=\"PA0\"\n\n[[exti]]\npin=\"PB1\"\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn invalid_exti_pin_is_reported() {
        let text = "[[exti]]\npin=\"PZ9\"\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(
            &conflicts[0],
            Conflict::IrqConflict { node, .. } if node == "PZ9"
        ));
    }

    #[test]
    fn priority_inversion_fires_when_dma_less_urgent() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=true\ndma_priority=5\nirq_priority=1\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        match &conflicts[0] {
            Conflict::IrqConflict {
                node,
                severity,
                reason,
            } => {
                assert_eq!(node, "USART2");
                assert_eq!(*severity, Severity::Warning);
                assert!(reason.contains('5') && reason.contains('1'), "{reason}");
            }
            other => panic!("expected IrqConflict, got {other:?}"),
        }
    }

    #[test]
    fn priority_not_inverted_is_clean() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=true\ndma_priority=1\nirq_priority=5\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn priority_inversion_skipped_when_one_key_absent() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=true\ndma_priority=5\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn exti_priority_inversion_fires_when_exti_less_urgent_on_shared_pin() {
        // EXTI on PA3 (priority 5) shares its pin with USART2's RX, whose
        // IRQ priority is 1 (more urgent) — the EXTI is less urgent.
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\nirq_priority=1\n\n[[exti]]\npin=\"PA3\"\npriority=5\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        match &conflicts[0] {
            Conflict::IrqConflict {
                node,
                severity,
                reason,
            } => {
                assert_eq!(node, "USART2");
                assert_eq!(*severity, Severity::Warning);
                assert!(reason.contains('5') && reason.contains('1'), "{reason}");
                assert!(reason.contains("PA3"), "{reason}");
            }
            other => panic!("expected IrqConflict, got {other:?}"),
        }
    }

    #[test]
    fn exti_priority_not_inverted_on_shared_pin_is_clean() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\nirq_priority=5\n\n[[exti]]\npin=\"PA3\"\npriority=1\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn exti_priority_inversion_skipped_when_pins_differ() {
        // EXTI is on PB0, unrelated to USART2's PA2/PA3 pins.
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\nirq_priority=1\n\n[[exti]]\npin=\"PB0\"\npriority=5\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn exti_priority_inversion_skipped_when_no_irq_priority_set() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\n\n[[exti]]\npin=\"PA3\"\npriority=5\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn exti_priority_inversion_skipped_when_peripheral_irq_not_enabled() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq_priority=1\n\n[[exti]]\npin=\"PA3\"\npriority=5\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn exti_priority_inversion_skipped_when_exti_priority_absent() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\nirq_priority=1\n\n[[exti]]\npin=\"PA3\"\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn family_parameterized_f411() {
        // UART4 doesn't exist on the F411; this same table would still error
        // (unmodeled, same as on F446 — UART4 is F446-only, so on F411 it's
        // unmodeled the way `PeripheralUnavailable` flags it at the solver
        // level. Here we confirm the IRQ map itself is family-aware: USART3
        // is modeled on F446 but not on F411.)
        let text = "[peripherals.usart3]\ntx=\"PB10\"\nrx=\"PB11\"\nirq=true\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
        let conflicts = validate_toml(text, &f411());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(
            &conflicts[0],
            Conflict::IrqConflict { node, .. } if node == "USART3"
        ));
    }
}
