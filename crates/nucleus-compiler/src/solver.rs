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

/// A single resolved conflict. Every variant is an error (it makes the config
/// un-buildable); `nucleus check` exits non-zero if any are present.
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

    // Clock-tree validation (M1) runs last, so clock conflicts sort after the
    // pin/AF/domain conflicts in the deterministic order.
    let tree = crate::clock_tree_for(&config.device.family);
    conflicts.extend(crate::clocks::validate(config, &tree));

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
}
