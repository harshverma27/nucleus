//! STM32 constraint database for the Nucleus toolchain.
//!
//! Exposes a normalized pin ↔ alternate-function ↔ peripheral model and the
//! lookup APIs the compiler and LSP build on. The data is currently a
//! hand-verified seed for the **STM32F446RE**; the full CMSIS Device Family
//! Pack parser that will populate it lives in [`pack`] and is not yet wired up.
//!
//! See the Phase 1 exit criteria in the project README.

use std::fmt;
use std::str::FromStr;

pub mod clock;
mod data;
pub mod dma;
pub mod irq;
pub mod pack;

/// A GPIO port on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Port {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
}

impl Port {
    /// The port's letter, e.g. `Port::A` → `'A'`.
    pub const fn letter(self) -> char {
        match self {
            Port::A => 'A',
            Port::B => 'B',
            Port::C => 'C',
            Port::D => 'D',
            Port::E => 'E',
            Port::F => 'F',
            Port::G => 'G',
            Port::H => 'H',
        }
    }

    const fn from_letter(c: char) -> Option<Port> {
        match c {
            'A' | 'a' => Some(Port::A),
            'B' | 'b' => Some(Port::B),
            'C' | 'c' => Some(Port::C),
            'D' | 'd' => Some(Port::D),
            'E' | 'e' => Some(Port::E),
            'F' | 'f' => Some(Port::F),
            'G' | 'g' => Some(Port::G),
            'H' | 'h' => Some(Port::H),
            _ => None,
        }
    }
}

/// A physical pin, e.g. `PA7` is `Port::A` pin `7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pin {
    pub port: Port,
    pub number: u8,
}

impl Pin {
    pub const fn new(port: Port, number: u8) -> Pin {
        Pin { port, number }
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P{}{}", self.port.letter(), self.number)
    }
}

/// Error returned when a string cannot be parsed as a [`Pin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePinError(String);

impl fmt::Display for ParsePinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid pin name: {:?} (expected e.g. \"PA7\")", self.0)
    }
}

impl std::error::Error for ParsePinError {}

impl FromStr for Pin {
    type Err = ParsePinError;

    fn from_str(s: &str) -> Result<Pin, ParsePinError> {
        let err = || ParsePinError(s.to_string());
        let rest = s
            .strip_prefix('P')
            .or_else(|| s.strip_prefix('p'))
            .ok_or_else(err)?;
        let mut chars = rest.chars();
        let port = chars.next().and_then(Port::from_letter).ok_or_else(err)?;
        let number: u8 = chars.as_str().parse().map_err(|_| err())?;
        if number > 15 {
            return Err(err());
        }
        Ok(Pin { port, number })
    }
}

/// One alternate-function mapping: a [`Pin`] driving a peripheral signal at a
/// given AF number (AF0–AF15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AfMapping {
    pub pin: Pin,
    pub af: u8,
    /// Peripheral instance, e.g. `"SPI1"`.
    pub peripheral: &'static str,
    /// Signal on that peripheral, e.g. `"MOSI"`.
    pub signal: &'static str,
}

/// A queryable constraint database for a single device.
#[derive(Debug, Clone, Copy)]
pub struct Database {
    entries: &'static [AfMapping],
}

impl Database {
    /// The hand-verified database for the NUCLEO-F446RE's STM32F446RE.
    pub const fn f446re() -> Database {
        Database {
            entries: data::F446RE,
        }
    }

    /// The database for the NUCLEO-F411RE's STM32F411RE, generated at build
    /// time from the vendored ST pin data (see [`pack`]).
    pub const fn f411re() -> Database {
        Database {
            entries: data::F411RE,
        }
    }

    /// Whether this device exposes `peripheral` at all (any AF mapping names it).
    /// Used by the solver to flag peripherals absent on the selected family.
    pub fn has_peripheral(&self, peripheral: &str) -> bool {
        self.entries.iter().any(|m| m.peripheral == peripheral)
    }

    /// Every mapping for `pin` at alternate function `af`.
    ///
    /// Returns an iterator because one (pin, AF) can carry several peripheral
    /// signals — e.g. on the F446 PA4/AF5 is both `SPI1_NSS` and `I2S1_WS`
    /// (SPI and I2S share the same hardware block and AF number).
    pub fn lookup(&self, pin: Pin, af: u8) -> impl Iterator<Item = &AfMapping> {
        self.entries
            .iter()
            .filter(move |m| m.pin == pin && m.af == af)
    }

    /// Every alternate-function mapping available on `pin`.
    pub fn alt_functions(&self, pin: Pin) -> impl Iterator<Item = &AfMapping> {
        self.entries.iter().filter(move |m| m.pin == pin)
    }

    /// Every physical pin that has at least one alternate function, sorted and
    /// deduplicated. Used by the LSP to offer pin-name completions.
    pub fn pins(&self) -> Vec<Pin> {
        let mut pins: Vec<Pin> = self.entries.iter().map(|m| m.pin).collect();
        pins.sort_unstable();
        pins.dedup();
        pins
    }

    /// Reverse lookup: the AF number that connects `pin` to
    /// `peripheral`'s `signal`, if any. Used by the constraint solver.
    pub fn find_af(&self, pin: Pin, peripheral: &str, signal: &str) -> Option<u8> {
        self.entries
            .iter()
            .find(|m| m.pin == pin && m.peripheral == peripheral && m.signal == signal)
            .map(|m| m.af)
    }

    /// Enumerate the candidate pins that carry `peripheral`'s `signal`,
    /// in deterministic order.
    ///
    /// Every physical pin that has at least one AF mapping to the given
    /// `(peripheral, signal)` pair is returned, sorted and deduplicated.
    /// Returns an empty vector if the family does not model the signal
    /// (the auto-router then skips it rather than guessing — never a false error,
    /// matching the `peripheral_bus` discipline).
    pub fn candidate_pins(&self, peripheral: &str, signal: &str) -> Vec<Pin> {
        let mut pins: Vec<Pin> = self
            .entries
            .iter()
            .filter(|m| m.peripheral == peripheral && m.signal == signal)
            .map(|m| m.pin)
            .collect();
        pins.sort_unstable();
        pins.dedup();
        pins
    }
}

#[cfg(test)]
mod tests;
