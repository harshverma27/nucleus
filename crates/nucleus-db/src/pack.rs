//! Parser for ST's open pin data (CubeMX XML) into the normalized
//! pin ↔ AF ↔ peripheral model, plus the patch table for known anomalies.
//!
//! This module is deliberately self-contained (no `crate::` types) because it
//! is also included by `build.rs` via `#[path]` to generate the compiled-in
//! database at build time. Sources live in `packdata/` (see its README).

/// One raw mapping parsed from the GPIO modes XML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMapping {
    /// Pin name, e.g. `"PA7"`.
    pub pin: String,
    /// Alternate function number, 0–15.
    pub af: u8,
    /// Peripheral instance, e.g. `"SPI1"`.
    pub peripheral: String,
    /// Signal on that peripheral, e.g. `"MOSI"`.
    pub signal: String,
}

/// Error parsing vendored pack data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// The XML structure was not what we expect from ST's pin data files.
    Malformed(String),
}

impl core::fmt::Display for PackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PackError::Malformed(what) => write!(f, "malformed pack data: {what}"),
        }
    }
}

impl std::error::Error for PackError {}

/// A recorded correction to upstream pack data. Every deviation from the
/// vendored XML lives here so it is traceable (Phase 1 exit criteria).
#[derive(Debug, Clone, Copy)]
pub enum Patch {
    /// Drop an upstream mapping that is wrong for this device.
    Remove {
        pin: &'static str,
        af: u8,
        peripheral: &'static str,
        signal: &'static str,
        reason: &'static str,
    },
    /// Add a mapping missing from upstream data.
    Add {
        pin: &'static str,
        af: u8,
        peripheral: &'static str,
        signal: &'static str,
        reason: &'static str,
    },
}

/// Known anomalies in the vendored STM32F4 pin data (shared across families;
/// each patch is keyed by exact `(pin, af, peripheral, signal)` so there is no
/// cross-family collision risk).
///
/// No entry-level corrections are needed yet; the generated table is
/// cross-validated against a hand-verified datasheet seed (see
/// `generated_db_agrees_with_hand_verified_seed`). Record any future
/// deviation here, never by editing `packdata/`.
///
/// Anomalies found so far are *structural* and handled (with tests) in
/// [`parse_gpio_modes`] rather than as per-entry patches:
/// - `"CEC"`: HDMI-CEC's signal name has no `PERIPH_SIGNAL` form; it
///   normalizes to peripheral `CEC`, signal `CEC`.
/// - `"PDR_ON"`: the modes file lists this power ball as a `GPIO_Pin`; any
///   entry that is not a `P<port><n>` pin is skipped.
/// - `"PI8"`: present in the family-wide modes file but not on the LQFP64
///   package; dropped by the package-pin filter in [`generate_table`].
pub const PATCHES: &[Patch] = &[];

/// Parse the GPIO IP modes XML (`GPIO-*_Modes.xml`) into raw mappings.
pub fn parse_gpio_modes(xml: &str) -> Result<Vec<RawMapping>, PackError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| PackError::Malformed(format!("gpio modes xml: {e}")))?;

    let mut mappings = Vec::new();
    for gpio_pin in elements_named(&doc, "GPIO_Pin") {
        let pin = required_attr(&gpio_pin, "Name")?;
        // Known anomaly: the modes file also lists non-GPIO entries (the
        // F446's PDR_ON ball); skip anything that isn't a P<port><n> pin.
        let Some(pin) = normalize_pin_name(pin) else {
            continue;
        };

        for pin_signal in gpio_pin
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "PinSignal")
        {
            let signal_name = required_attr(&pin_signal, "Name")?;
            let Some(af) = gpio_af_value(&pin_signal) else {
                // PinSignal without a GPIO_AF parameter: not an AF mux entry.
                continue;
            };
            let af = af
                .strip_prefix("GPIO_AF")
                .and_then(|s| s.split('_').next())
                .and_then(|s| s.parse::<u8>().ok())
                .filter(|af| *af <= 15)
                .ok_or_else(|| PackError::Malformed(format!("bad GPIO_AF value {af:?}")))?;

            // "SPI1_MOSI" → ("SPI1", "MOSI"); "SYS_JTMS-SWDIO" → ("SYS", "JTMS-SWDIO").
            // Known anomaly: peripheral-only names (the F446's HDMI-CEC is
            // just "CEC") normalize to peripheral == signal.
            let (peripheral, signal) = signal_name
                .split_once('_')
                .unwrap_or((signal_name, signal_name));

            mappings.push(RawMapping {
                pin: pin.to_string(),
                af,
                peripheral: peripheral.to_string(),
                signal: signal.to_string(),
            });
        }
    }
    Ok(mappings)
}

/// Parse the MCU package XML and return the GPIO pins that physically exist
/// on the package, normalized (e.g. `"PC14-OSC32_IN"` → `"PC14"`).
pub fn parse_package_pins(xml: &str) -> Result<Vec<String>, PackError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| PackError::Malformed(format!("mcu xml: {e}")))?;

    Ok(elements_named(&doc, "Pin")
        .filter_map(|pin| {
            let name = pin.attribute("Name")?;
            // Power/reset/boot pins (VBAT, NRST, VSS...) are not GPIOs.
            normalize_pin_name(name).map(str::to_string)
        })
        .collect())
}

/// Apply the patch table to parsed mappings.
pub fn apply_patches(mappings: &mut Vec<RawMapping>, patches: &[Patch]) {
    for patch in patches {
        match *patch {
            Patch::Remove {
                pin,
                af,
                peripheral,
                signal,
                reason: _,
            } => mappings.retain(|m| {
                !(m.pin == pin && m.af == af && m.peripheral == peripheral && m.signal == signal)
            }),
            Patch::Add {
                pin,
                af,
                peripheral,
                signal,
                reason: _,
            } => mappings.push(RawMapping {
                pin: pin.to_string(),
                af,
                peripheral: peripheral.to_string(),
                signal: signal.to_string(),
            }),
        }
    }
}

/// Render mappings (filtered to `package_pins`, sorted, deduplicated) as Rust
/// source for inclusion by `data.rs`. Output is byte-deterministic.
pub fn generate_table(
    mappings: &[RawMapping],
    package_pins: &[String],
    const_name: &str,
) -> String {
    let mut rows: Vec<(char, u8, &RawMapping)> = mappings
        .iter()
        .filter(|m| package_pins.contains(&m.pin))
        .filter_map(|m| {
            let (port, number) = pin_sort_key(&m.pin)?;
            Some((port, number, m))
        })
        .collect();
    rows.sort_by_key(|(port, number, m)| {
        (*port, *number, m.af, m.peripheral.clone(), m.signal.clone())
    });
    rows.dedup_by(|a, b| a.2 == b.2);

    let mut out = format!(
        "// @generated by build.rs from packdata/ — do not edit.\n\
         pub(crate) const {const_name}: &[AfMapping] = &[\n",
    );
    for (port, number, m) in rows {
        out.push_str(&format!(
            "    map(Port::{port}, {number}, {af}, \"{peripheral}\", \"{signal}\"),\n",
            af = m.af,
            peripheral = m.peripheral,
            signal = m.signal,
        ));
    }
    out.push_str("];\n");
    out
}

/// `"PC14-OSC32_IN"` → `Some("PC14")`; `"VBAT"`/`"NRST"` → `None`.
fn normalize_pin_name(name: &str) -> Option<&str> {
    let base = name.split(['-', '/', ' ']).next().unwrap_or(name);
    pin_sort_key(base).map(|_| base)
}

/// `"PA7"` → `Some(('A', 7))` for sorting and validation.
fn pin_sort_key(pin: &str) -> Option<(char, u8)> {
    let rest = pin.strip_prefix('P')?;
    let port = rest.chars().next().filter(char::is_ascii_uppercase)?;
    let number: u8 = rest[1..].parse().ok().filter(|n| *n <= 15)?;
    Some((port, number))
}

/// All descendant elements with local name `name` (the files use a default
/// namespace, so match on local names).
fn elements_named<'a>(
    doc: &'a roxmltree::Document<'a>,
    name: &'static str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> {
    doc.descendants()
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
}

fn required_attr<'a>(
    node: &roxmltree::Node<'a, '_>,
    attr: &'static str,
) -> Result<&'a str, PackError> {
    node.attribute(attr).ok_or_else(|| {
        PackError::Malformed(format!(
            "<{}> missing {attr} attribute",
            node.tag_name().name()
        ))
    })
}

/// The `GPIO_AF` PossibleValue text under a PinSignal, if present.
fn gpio_af_value<'a>(pin_signal: &roxmltree::Node<'a, '_>) -> Option<&'a str> {
    pin_signal
        .children()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "SpecificParameter"
                && n.attribute("Name") == Some("GPIO_AF")
        })?
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "PossibleValue")?
        .text()
}
