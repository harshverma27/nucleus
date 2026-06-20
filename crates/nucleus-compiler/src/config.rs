//! Parsing of the `stm32.toml` project configuration.
//!
//! The format is documented in the project README. This module owns only the
//! *syntactic* layer: turn TOML text into a typed [`Config`]. It does no
//! hardware validation — that is the solver's job (see [`crate::solver`]).
//!
//! Peripheral tables carry a mix of pin assignments (`tx = "PA2"`) and tuning
//! parameters (`baud = 115200`), and the set of keys differs per peripheral
//! kind. Rather than hard-code every peripheral struct here, each
//! `[peripherals.<name>]` table is kept as a raw key→value map and interpreted
//! by the solver against the [`crate::model`] role tables. This keeps the
//! parser stable as new peripherals are added.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A parsed `stm32.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub device: Device,
    #[serde(default)]
    pub build: Build,
    /// `[peripherals.<instance>]` tables, keyed by instance name as written
    /// (e.g. `"usart2"`). Order is normalized to lexical for deterministic
    /// diagnostics.
    #[serde(default)]
    pub peripherals: BTreeMap<String, Peripheral>,
    /// `[clocks]` — which device bus domains are enabled. Absent means "all
    /// enabled" so a config that omits the section never trips the clock check.
    #[serde(default)]
    pub clocks: Clocks,
    #[serde(default)]
    pub trace: Trace,
    /// `[[exti]]` entries — external interrupt pin definitions.
    #[serde(default)]
    pub exti: Vec<ExtiPin>,
    /// `[[test]]` entries — declarative HIL test cases (M6).
    #[serde(default)]
    pub test: Vec<TestCase>,
}

/// The `[device]` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub clock_hz: Option<u64>,
}

/// The `[build]` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    #[serde(default)]
    pub toolchain: Option<String>,
    #[serde(default)]
    pub optimization: Option<String>,
}

/// One `[peripherals.<instance>]` table: a raw bag of keys. Pin roles are
/// string values; tuning parameters are everything else.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct Peripheral(pub BTreeMap<String, toml::Value>);

impl Peripheral {
    /// The string value of `key`, if present and a string (i.e. a pin role).
    pub fn pin_str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(toml::Value::as_str)
    }
}

/// The `[clocks]` section.
///
/// The boolean bus fields default to `true` (enabled) so an omitted field never
/// produces a false "clock disabled" error. The clock-tree fields (PLL dividers,
/// prescalers, source selection) are all `Option`: when absent, the M1 solver
/// substitutes the family's known-good NUCLEO power-on configuration, so a config
/// that omits `[clocks]` entirely validates clean.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clocks {
    #[serde(default = "enabled")]
    pub ahb1: bool,
    #[serde(default = "enabled")]
    pub apb1: bool,
    #[serde(default = "enabled")]
    pub apb2: bool,

    /// SYSCLK source: `"hsi"`, `"hse"`, or `"pll"`. Default `"pll"`.
    #[serde(default)]
    pub source: Option<String>,
    /// Oscillator feeding the PLL: `"hsi"` or `"hse"`. Default `"hse"`.
    #[serde(default)]
    pub pll_source: Option<String>,
    #[serde(default)]
    pub pll_m: Option<u32>,
    #[serde(default)]
    pub pll_n: Option<u32>,
    #[serde(default)]
    pub pll_p: Option<u32>,
    #[serde(default)]
    pub pll_q: Option<u32>,
    #[serde(default)]
    pub ahb_prescaler: Option<u32>,
    #[serde(default)]
    pub apb1_prescaler: Option<u32>,
    #[serde(default)]
    pub apb2_prescaler: Option<u32>,
}

fn enabled() -> bool {
    true
}

impl Default for Clocks {
    fn default() -> Clocks {
        Clocks {
            ahb1: true,
            apb1: true,
            apb2: true,
            source: None,
            pll_source: None,
            pll_m: None,
            pll_n: None,
            pll_p: None,
            pll_q: None,
            ahb_prescaler: None,
            apb1_prescaler: None,
            apb2_prescaler: None,
        }
    }
}

/// The `[trace]` section. Parsed for completeness; not validated in Phase 2.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub swo_freq: Option<u64>,
    #[serde(default)]
    pub variables: Vec<TraceVariable>,
}

/// One `[[trace.variables]]` entry.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceVariable {
    pub name: String,
    pub port: u8,
    #[serde(rename = "type")]
    pub ty: String,
}

/// One `[[exti]]` entry.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtiPin {
    pub pin: String,
    #[serde(default)]
    pub priority: Option<u8>,
}

/// One `[[test]]` entry — a declarative HIL assertion (M6).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, rename = "test")]
pub struct TestCase {
    pub name: String,
    pub assertion: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub backend: Option<String>, // "qemu" | "hardware" | "both"; None = both
}

fn default_timeout_ms() -> u64 {
    1000
}

/// Error returned when `stm32.toml` text is not valid TOML or violates the schema.
#[derive(Debug)]
pub struct ParseError(toml::de::Error);

impl ParseError {
    /// The byte span in the source text the error refers to, if the underlying
    /// TOML parser recorded one. Used by the LSP to place a diagnostic range.
    pub fn span(&self) -> Option<std::ops::Range<usize>> {
        self.0.span()
    }

    /// The bare error message (without the `invalid stm32.toml:` prefix).
    pub fn message(&self) -> &str {
        self.0.message()
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid stm32.toml: {}", self.0.message())
    }
}

impl std::error::Error for ParseError {}

/// Parse `stm32.toml` text into a [`Config`].
pub fn parse(text: &str) -> Result<Config, ParseError> {
    toml::from_str(text).map_err(ParseError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_readme_example() {
        let cfg = parse(
            r#"
[device]
family = "STM32F446RE"
board = "NUCLEO-F446RE"
clock_hz = 180_000_000

[peripherals.usart2]
tx = "PA2"
rx = "PA3"
baud = 115200

[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
nss = "PA4"
mode = 0
"#,
        )
        .unwrap();

        assert_eq!(cfg.device.family, "STM32F446RE");
        assert_eq!(cfg.peripherals.len(), 2);
        assert_eq!(cfg.peripherals["usart2"].pin_str("tx"), Some("PA2"));
        // Tuning params are not pin roles.
        assert_eq!(cfg.peripherals["spi1"].pin_str("mode"), None);
        // Clocks default to all-enabled.
        assert!(cfg.clocks.apb1 && cfg.clocks.apb2 && cfg.clocks.ahb1);
    }

    #[test]
    fn rejects_unknown_top_level_section() {
        assert!(parse("[nonsense]\nfoo = 1\n").is_err());
    }

    #[test]
    fn clocks_section_can_disable_a_bus() {
        let cfg = parse("[clocks]\napb1 = false\n").unwrap();
        assert!(!cfg.clocks.apb1);
        assert!(cfg.clocks.apb2); // unspecified -> enabled
    }

    #[test]
    fn parses_exti_entries() {
        let cfg = parse(
            r#"
[[exti]]
pin = "PA0"
priority = 5

[[exti]]
pin = "PB1"
"#,
        )
        .unwrap();

        assert_eq!(cfg.exti.len(), 2);
        assert_eq!(cfg.exti[0].pin, "PA0");
        assert_eq!(cfg.exti[0].priority, Some(5));
        assert_eq!(cfg.exti[1].pin, "PB1");
        assert_eq!(cfg.exti[1].priority, None);
    }

    #[test]
    fn parses_minimal_test_block() {
        let cfg = parse(
            r#"
[[test]]
name = "uart2_echo"
assertion = "UART2 echoes 'ping' within 10ms"
"#,
        )
        .unwrap();

        assert_eq!(cfg.test.len(), 1);
        assert_eq!(cfg.test[0].name, "uart2_echo");
        assert_eq!(cfg.test[0].assertion, "UART2 echoes 'ping' within 10ms");
        assert_eq!(cfg.test[0].timeout_ms, 1000);
        assert_eq!(cfg.test[0].backend, None);
    }

    #[test]
    fn parses_fully_specified_test_block() {
        let cfg = parse(
            r#"
[[test]]
name = "uart2_echo"
assertion = "UART2 echoes 'ping' within 10ms"
timeout_ms = 100
backend = "both"
"#,
        )
        .unwrap();

        assert_eq!(cfg.test.len(), 1);
        assert_eq!(cfg.test[0].name, "uart2_echo");
        assert_eq!(cfg.test[0].assertion, "UART2 echoes 'ping' within 10ms");
        assert_eq!(cfg.test[0].timeout_ms, 100);
        assert_eq!(cfg.test[0].backend, Some("both".to_string()));
    }

    #[test]
    fn rejects_unknown_field_in_test_block() {
        let result = parse(
            r#"
[[test]]
name = "uart2_echo"
assertion = "UART2 echoes 'ping' within 10ms"
bogus = 1
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parses_multiple_test_blocks_in_document_order() {
        let cfg = parse(
            r#"
[[test]]
name = "first"
assertion = "pin PA5 is high within 10ms"

[[test]]
name = "second"
assertion = "pin PA6 is low within 20ms"
"#,
        )
        .unwrap();

        assert_eq!(cfg.test.len(), 2);
        assert_eq!(cfg.test[0].name, "first");
        assert_eq!(cfg.test[1].name, "second");
    }
}
