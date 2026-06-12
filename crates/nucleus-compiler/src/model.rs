//! The static peripheral model: how `stm32.toml` keys map to database signals,
//! which pins a peripheral requires, and which bus clock domain feeds it.
//!
//! This is intentionally a small hand-maintained table rather than something
//! derived from the pack data: the pack knows pin↔signal wiring, but not the
//! *ergonomic* config-key names (`tx`, `mosi`, `sda`) nor the required-vs-optional
//! distinction, which are Nucleus product decisions.

/// A device bus clock domain. Phase 2 does only "is this bus enabled?" checking
/// (per the scope rules — no full clock-tree solving).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Ahb1,
    Apb1,
    Apb2,
}

impl Bus {
    pub fn name(self) -> &'static str {
        match self {
            Bus::Ahb1 => "AHB1",
            Bus::Apb1 => "APB1",
            Bus::Apb2 => "APB2",
        }
    }
}

/// One configurable pin on a peripheral: the `stm32.toml` key, the database
/// signal name it resolves to, and whether the peripheral is unusable without it.
#[derive(Debug, Clone, Copy)]
pub struct Role {
    /// The key as written in the config, e.g. `"tx"`.
    pub key: &'static str,
    /// The database signal name, e.g. `"TX"`.
    pub signal: &'static str,
    /// Whether omitting this pin is an error (vs. a legitimately optional line).
    pub required: bool,
}

const fn req(key: &'static str, signal: &'static str) -> Role {
    Role {
        key,
        signal,
        required: true,
    }
}

const fn opt(key: &'static str, signal: &'static str) -> Role {
    Role {
        key,
        signal,
        required: false,
    }
}

/// The pin roles for a peripheral kind, in canonical order.
struct Kind {
    roles: &'static [Role],
}

const USART: Kind = Kind {
    roles: &[
        req("tx", "TX"),
        req("rx", "RX"),
        opt("cts", "CTS"),
        opt("rts", "RTS"),
        opt("ck", "CK"),
    ],
};

const SPI: Kind = Kind {
    roles: &[
        req("mosi", "MOSI"),
        req("miso", "MISO"),
        req("sck", "SCK"),
        opt("nss", "NSS"),
    ],
};

const I2C: Kind = Kind {
    roles: &[req("sda", "SDA"), req("scl", "SCL"), opt("smba", "SMBA")],
};

const TIM: Kind = Kind {
    roles: &[
        opt("channel1", "CH1"),
        opt("channel2", "CH2"),
        opt("channel3", "CH3"),
        opt("channel4", "CH4"),
    ],
};

/// The pin roles for the peripheral instance named `instance` (e.g. `"usart2"`),
/// or `None` if the kind is not modelled. The match is on the alphabetic prefix,
/// so `usart2` and `usart3` share one role table.
pub fn roles_for(instance: &str) -> Option<&'static [Role]> {
    let kind = instance.trim_end_matches(|c: char| c.is_ascii_digit());
    let kind = match kind {
        "usart" | "uart" => &USART,
        "spi" => &SPI,
        "i2c" | "fmpi2c" => &I2C,
        "tim" => &TIM,
        _ => return None,
    };
    Some(kind.roles)
}

/// The database peripheral name for a config instance name, e.g.
/// `"usart2"` → `"USART2"`. Nucleus simply upper-cases the instance name; the
/// database uses the same convention as ST's pin data.
pub fn peripheral_name(instance: &str) -> String {
    instance.to_ascii_uppercase()
}

/// The bus clock domain that feeds `peripheral` on the STM32F446RE, or `None`
/// if unknown (in which case the clock check is skipped — never a false error).
///
/// Source: STM32F446xx reference manual (RM0390) RCC peripheral-clock-enable
/// registers. Only the peripherals Nucleus models in Phase 2 need be exact;
/// the table errs toward `None` rather than guessing.
pub fn peripheral_bus(peripheral: &str) -> Option<Bus> {
    let p = peripheral;
    let bus = match p {
        // APB2
        "USART1" | "USART6" | "SPI1" | "SPI4" | "SDIO" | "ADC1" | "ADC2" | "ADC3" | "SAI1"
        | "SAI2" | "TIM1" | "TIM8" | "TIM9" | "TIM10" | "TIM11" => Bus::Apb2,
        // APB1
        "USART2" | "USART3" | "UART4" | "UART5" | "SPI2" | "SPI3" | "I2C1" | "I2C2" | "I2C3"
        | "FMPI2C1" | "CAN1" | "CAN2" | "SPDIFRX" | "CEC" | "TIM2" | "TIM3" | "TIM4" | "TIM5"
        | "TIM6" | "TIM7" | "TIM12" | "TIM13" | "TIM14" => Bus::Apb1,
        // AHB1 / OTG
        "DCMI" | "QUADSPI" | "FMC" => Bus::Ahb1,
        _ => return None,
    };
    Some(bus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_tables_resolve_by_prefix() {
        assert!(roles_for("usart2").is_some());
        assert!(roles_for("spi1").is_some());
        assert!(roles_for("i2c3").is_some());
        assert!(roles_for("tim2").is_some());
        assert!(roles_for("wibble9").is_none());
    }

    #[test]
    fn spi_required_and_optional() {
        let roles = roles_for("spi1").unwrap();
        let nss = roles.iter().find(|r| r.key == "nss").unwrap();
        let sck = roles.iter().find(|r| r.key == "sck").unwrap();
        assert!(!nss.required);
        assert!(sck.required);
    }

    #[test]
    fn buses_match_f446() {
        assert_eq!(peripheral_bus("SPI1"), Some(Bus::Apb2));
        assert_eq!(peripheral_bus("USART2"), Some(Bus::Apb1));
        assert_eq!(peripheral_bus("I2C1"), Some(Bus::Apb1));
        assert_eq!(peripheral_bus("MADEUP"), None);
    }
}
