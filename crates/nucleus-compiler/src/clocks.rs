//! The clock-tree solver (Nucleus v2 milestone M1).
//!
//! Turns the v1 "is the bus clock enabled?" boolean check into real frequency
//! math: it [`resolve`]s a `[clocks]` config against the family's
//! [`nucleus_db::clock::ClockTree`] model into the actual frequency at every
//! node, and [`validate`]s those frequencies against the silicon's limits — the
//! over-clocked buses, out-of-range PLL settings, and unreachable peripheral
//! rates that CubeMX silently accepts.
//!
//! Everything here is **pure and synchronous** (config + model → frequencies /
//! conflicts), so it unit-tests trivially and the LSP picks it up for free. All
//! limits come from the model, never hard-coded, so the F446 and F411 differ
//! automatically.

use nucleus_db::clock::{Bus, ClockTree, Oscillator, SysclkSource};

use crate::config::{Clocks, Config};
use crate::solver::Conflict;

/// The resolved frequency at every clock-tree node, in hertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedClocks {
    pub sysclk_hz: u32,
    pub hclk_hz: u32,
    pub pclk1_hz: u32,
    pub pclk2_hz: u32,
    pub apb1_timer_hz: u32,
    pub apb2_timer_hz: u32,
    /// PLL VCO input (after the M divider); `0` when SYSCLK is not the PLL.
    pub pll_vco_in_hz: u32,
    /// PLL VCO output (after the N multiplier); `0` when SYSCLK is not the PLL.
    pub pll_vco_out_hz: u32,
}

/// The actual frequency one configured peripheral receives — its derived bus
/// clock, or the doubled timer clock for a `TIM*` on an APB bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeripheralFreq {
    pub bus: Bus,
    pub hz: u32,
}

/// Why a `[clocks]` config could not be turned into frequencies at all. Surfaced
/// as a [`Conflict::ClockConstraint`] by [`validate`]; never panics.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolveError {
    UnknownSource { field: &'static str, value: String },
    ZeroDivider { node: &'static str },
}

/// The concrete clock configuration, with every optional `[clocks]` field folded
/// to a value — absent fields filled from the family's known-good defaults.
#[derive(Debug, Clone, Copy)]
struct Effective {
    source: SysclkSource,
    pll_source: Oscillator,
    pll_m: u32,
    pll_n: u32,
    pll_p: u32,
    pll_q: Option<u32>,
    ahb_presc: u32,
    apb1_presc: u32,
    apb2_presc: u32,
}

fn parse_sysclk_source(s: &str) -> Option<SysclkSource> {
    match s.to_ascii_lowercase().as_str() {
        "hsi" => Some(SysclkSource::Hsi),
        "hse" => Some(SysclkSource::Hse),
        "pll" => Some(SysclkSource::Pll),
        _ => None,
    }
}

fn parse_pll_source(s: &str) -> Option<Oscillator> {
    match s.to_ascii_lowercase().as_str() {
        "hsi" => Some(Oscillator::Hsi),
        "hse" => Some(Oscillator::Hse),
        _ => None,
    }
}

fn osc_hz(tree: &ClockTree, osc: Oscillator) -> u32 {
    tree.oscillator(osc).map(|o| o.nominal_hz).unwrap_or(0)
}

/// The family's known-good default PLL/prescaler selections, computed from the
/// model so each family lands exactly at its own limit (F446 → 180 MHz on
/// PCLK-legal prescalers, F411 → 100 MHz). No silicon constant is hard-coded.
fn default_effective(tree: &ClockTree) -> Result<Effective, ResolveError> {
    let hse = osc_hz(tree, Oscillator::Hse);
    let limits = tree.limits();
    // Target 1 MHz VCO input: M = HSE / 1 MHz (8 for an 8 MHz HSE).
    let pll_m = (hse / 1_000_000).max(tree.pll().m.min);
    let pll_p = 2;
    let vco_in = hse / pll_m;
    // A family with no HSE modeled (or an HSE below 1 MHz, paired with the
    // PLL's own minimum M) leaves vco_in at 0 — the default PLL setup always
    // sources from HSE, so there's no default to fall back to.
    if vco_in == 0 {
        return Err(ResolveError::ZeroDivider { node: "PLL" });
    }
    // N so that SYSCLK = (vco_in * N) / P == max SYSCLK.
    let pll_n = limits.max_sysclk_hz * pll_p / vco_in;

    let ahb_presc = 1;
    let apb1_presc = smallest_presc_within(tree, Bus::Apb1, limits.max_sysclk_hz / ahb_presc);
    let apb2_presc = smallest_presc_within(tree, Bus::Apb2, limits.max_sysclk_hz / ahb_presc);

    Ok(Effective {
        source: SysclkSource::Pll,
        pll_source: Oscillator::Hse,
        pll_m,
        pll_n,
        pll_p,
        pll_q: Some(tree.pll().q.min.max(2)),
        ahb_presc,
        apb1_presc,
        apb2_presc,
    })
}

/// The smallest legal prescaler for `bus` such that `hclk / presc` is within that
/// bus's limit — so the default config never self-violates.
fn smallest_presc_within(tree: &ClockTree, bus: Bus, hclk: u32) -> u32 {
    let limit = match bus {
        Bus::Apb1 => tree.limits().max_apb1_hz,
        Bus::Apb2 => tree.limits().max_apb2_hz,
        Bus::Ahb1 => tree.limits().max_ahb_hz,
    };
    let mut chosen = *tree.prescalers(bus).last().unwrap_or(&1);
    for &p in tree.prescalers(bus) {
        if p != 0 && hclk / p <= limit {
            chosen = p;
            break;
        }
    }
    chosen
}

/// Fold the optional `[clocks]` fields onto the family defaults. Source strings
/// that don't parse are reported via [`ResolveError`] so they don't cascade.
fn effective(clocks: &Clocks, tree: &ClockTree) -> Result<Effective, ResolveError> {
    let mut eff = default_effective(tree)?;

    if let Some(s) = &clocks.source {
        eff.source = parse_sysclk_source(s).ok_or_else(|| ResolveError::UnknownSource {
            field: "SYSCLK",
            value: s.clone(),
        })?;
    }
    if let Some(s) = &clocks.pll_source {
        eff.pll_source = parse_pll_source(s).ok_or_else(|| ResolveError::UnknownSource {
            field: "PLL",
            value: s.clone(),
        })?;
    }
    if let Some(v) = clocks.pll_m {
        eff.pll_m = v;
    }
    if let Some(v) = clocks.pll_n {
        eff.pll_n = v;
    }
    if let Some(v) = clocks.pll_p {
        eff.pll_p = v;
    }
    if clocks.pll_q.is_some() {
        eff.pll_q = clocks.pll_q;
    }
    if let Some(v) = clocks.ahb_prescaler {
        eff.ahb_presc = v;
    }
    if let Some(v) = clocks.apb1_prescaler {
        eff.apb1_presc = v;
    }
    if let Some(v) = clocks.apb2_prescaler {
        eff.apb2_presc = v;
    }
    Ok(eff)
}

/// Compute the resolved frequency at every clock-tree node. Pure integer math,
/// mirroring the RCC dividers; guards every divisor so it never panics.
fn resolve_effective(eff: &Effective, tree: &ClockTree) -> Result<ResolvedClocks, ResolveError> {
    let (sysclk, vco_in, vco_out) = match eff.source {
        SysclkSource::Hsi => (osc_hz(tree, Oscillator::Hsi), 0, 0),
        SysclkSource::Hse => (osc_hz(tree, Oscillator::Hse), 0, 0),
        SysclkSource::Pll => {
            if eff.pll_m == 0 {
                return Err(ResolveError::ZeroDivider { node: "PLL" });
            }
            if eff.pll_p == 0 {
                return Err(ResolveError::ZeroDivider { node: "PLL" });
            }
            let input = osc_hz(tree, eff.pll_source);
            let vco_in = input / eff.pll_m;
            let vco_out = vco_in * eff.pll_n;
            (vco_out / eff.pll_p, vco_in, vco_out)
        }
    };

    if eff.ahb_presc == 0 {
        return Err(ResolveError::ZeroDivider { node: "AHB" });
    }
    if eff.apb1_presc == 0 {
        return Err(ResolveError::ZeroDivider { node: "APB1" });
    }
    if eff.apb2_presc == 0 {
        return Err(ResolveError::ZeroDivider { node: "APB2" });
    }

    let hclk = sysclk / eff.ahb_presc;
    let pclk1 = hclk / eff.apb1_presc;
    let pclk2 = hclk / eff.apb2_presc;

    Ok(ResolvedClocks {
        sysclk_hz: sysclk,
        hclk_hz: hclk,
        pclk1_hz: pclk1,
        pclk2_hz: pclk2,
        apb1_timer_hz: pclk1 * ClockTree::timer_multiplier(Bus::Apb1, eff.apb1_presc),
        apb2_timer_hz: pclk2 * ClockTree::timer_multiplier(Bus::Apb2, eff.apb2_presc),
        pll_vco_in_hz: vco_in,
        pll_vco_out_hz: vco_out,
    })
}

/// Resolve a `[clocks]` config into frequencies (public, pure entry point).
/// Returns `None` only when the config cannot be resolved at all (an unknown
/// source string or a zero divider) — those are reported as conflicts elsewhere.
pub fn resolve(clocks: &Clocks, tree: &ClockTree) -> Option<ResolvedClocks> {
    let eff = effective(clocks, tree).ok()?;
    resolve_effective(&eff, tree).ok()
}

/// The actual frequency a peripheral instance receives, or `None` when the family
/// does not model its bus (the caller then skips it — never a false error).
pub fn peripheral_freq(
    peripheral: &str,
    tree: &ClockTree,
    r: &ResolvedClocks,
) -> Option<PeripheralFreq> {
    let bus = tree.peripheral_bus(peripheral)?;
    let is_timer = peripheral.starts_with("TIM");
    let hz = match bus {
        Bus::Ahb1 => r.hclk_hz,
        Bus::Apb1 if is_timer => r.apb1_timer_hz,
        Bus::Apb1 => r.pclk1_hz,
        Bus::Apb2 if is_timer => r.apb2_timer_hz,
        Bus::Apb2 => r.pclk2_hz,
    };
    Some(PeripheralFreq { bus, hz })
}

// --- formatting helpers ---------------------------------------------------

/// Format a frequency in MHz, dropping a trailing `.0` (e.g. `45_000_000` →
/// `"45"`, `22_500_000` → `"22.5"`).
fn mhz(hz: u32) -> String {
    if hz % 1_000_000 == 0 {
        format!("{}", hz / 1_000_000)
    } else {
        let tenths = (hz as u64 * 10 / 1_000_000) as u32;
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

fn set_str(values: &[u32]) -> String {
    let parts: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("{{{}}}", parts.join(", "))
}

fn cc(node: &str, reason: String) -> Conflict {
    Conflict::ClockConstraint {
        node: node.to_string(),
        reason,
    }
}

/// Validate a config's clock tree against the family model, returning one
/// [`Conflict::ClockConstraint`] per offending node in a fixed, deterministic
/// order. Called by the solver after the pin/clock-domain checks.
pub fn validate(config: &Config, tree: &ClockTree) -> Vec<Conflict> {
    let clocks = &config.clocks;
    let mut out = Vec::new();

    // Fold to the effective config; an unknown source string is terminal (no
    // cascade) since we cannot compute frequencies from it.
    let eff = match effective(clocks, tree) {
        Ok(eff) => eff,
        Err(ResolveError::UnknownSource { field, value }) => {
            let (node, expected) = match field {
                "PLL" => ("PLL", "hsi or hse"),
                _ => ("SYSCLK", "hsi, hse, or pll"),
            };
            out.push(cc(
                node,
                format!("unknown {field} source {value:?} (expected {expected})"),
            ));
            return out;
        }
        Err(ResolveError::ZeroDivider { node }) => {
            out.push(cc(node, format!("{node} divider must not be zero")));
            return out;
        }
    };

    // 1. PLL divider legality (only meaningful when the PLL drives SYSCLK).
    let pll = tree.pll();
    if eff.source == SysclkSource::Pll {
        if !pll.m.contains(eff.pll_m) {
            out.push(cc(
                "PLL",
                format!(
                    "PLL M = {} is outside the valid {}\u{2013}{} range",
                    eff.pll_m, pll.m.min, pll.m.max
                ),
            ));
        }
        if !pll.n.contains(eff.pll_n) {
            out.push(cc(
                "PLL",
                format!(
                    "PLL N = {} is outside the valid {}\u{2013}{} range",
                    eff.pll_n, pll.n.min, pll.n.max
                ),
            ));
        }
        if !pll.p.contains(&eff.pll_p) {
            out.push(cc(
                "PLL",
                format!(
                    "PLL P = {} is not one of the allowed dividers {}",
                    eff.pll_p,
                    set_str(pll.p)
                ),
            ));
        }
        if let Some(q) = eff.pll_q {
            if !pll.q.contains(q) {
                out.push(cc(
                    "PLL",
                    format!(
                        "PLL Q = {} is outside the valid {}\u{2013}{} range",
                        q, pll.q.min, pll.q.max
                    ),
                ));
            }
        }
    }

    // 2. Prescaler legality (independent of SYSCLK source).
    for (node, presc, bus) in [
        ("AHB", eff.ahb_presc, Bus::Ahb1),
        ("APB1", eff.apb1_presc, Bus::Apb1),
        ("APB2", eff.apb2_presc, Bus::Apb2),
    ] {
        if !tree.prescalers(bus).contains(&presc) {
            out.push(cc(
                node,
                format!(
                    "{node} prescaler {presc} is not one of the allowed values {}",
                    set_str(tree.prescalers(bus))
                ),
            ));
        }
    }

    // If any structural divider is illegal, stop before frequency checks — the
    // numbers would be meaningless and would emit confusing cascades.
    if !out.is_empty() {
        return out;
    }

    let r = match resolve_effective(&eff, tree) {
        Ok(r) => r,
        Err(ResolveError::ZeroDivider { node }) => {
            out.push(cc(node, format!("{node} divider must not be zero")));
            return out;
        }
        Err(ResolveError::UnknownSource { .. }) => return out,
    };

    // 3. PLL VCO ranges (only when the PLL drives SYSCLK).
    if eff.source == SysclkSource::Pll {
        if !pll.vco_in.contains(r.pll_vco_in_hz) {
            out.push(cc(
                "PLL",
                format!(
                    "PLL VCO input = {} MHz is outside the {}\u{2013}{} MHz range",
                    mhz(r.pll_vco_in_hz),
                    mhz(pll.vco_in.min_hz),
                    mhz(pll.vco_in.max_hz)
                ),
            ));
        }
        if !pll.vco_out.contains(r.pll_vco_out_hz) {
            out.push(cc(
                "PLL",
                format!(
                    "PLL VCO output = {} MHz is outside the {}\u{2013}{} MHz range",
                    mhz(r.pll_vco_out_hz),
                    mhz(pll.vco_out.min_hz),
                    mhz(pll.vco_out.max_hz)
                ),
            ));
        }
    }

    // 4. Silicon limits (over-clock), fixed order SYSCLK, AHB, APB1, APB2.
    let limits = tree.limits();
    for (node, freq, limit) in [
        ("SYSCLK", r.sysclk_hz, limits.max_sysclk_hz),
        ("AHB", r.hclk_hz, limits.max_ahb_hz),
        ("APB1", r.pclk1_hz, limits.max_apb1_hz),
        ("APB2", r.pclk2_hz, limits.max_apb2_hz),
    ] {
        if freq > limit {
            out.push(cc(
                node,
                format!(
                    "{node} = {} MHz exceeds the {} MHz limit",
                    mhz(freq),
                    mhz(limit)
                ),
            ));
        }
    }

    // 5. Per-peripheral derived-rate reachability (UART baud). Deterministic via
    // the BTreeMap iteration order.
    for (instance, table) in &config.peripherals {
        let name = crate::model::peripheral_name(instance);
        if !(name.starts_with("USART") || name.starts_with("UART")) {
            continue;
        }
        let Some(baud) = table.0.get("baud").and_then(toml::Value::as_integer) else {
            continue;
        };
        if baud <= 0 {
            continue;
        }
        let Some(pf) = peripheral_freq(&name, tree, &r) else {
            continue;
        };
        if let Some(realized) = unreachable_baud(pf.hz, baud as u32) {
            let err_pct = pct_error(realized, baud as u32);
            out.push(cc(
                &name,
                format!(
                    "{baud} baud is unreachable from {} MHz {} (closest {} baud, {} error exceeds 2% tolerance)",
                    mhz(pf.hz),
                    pf.bus.name(),
                    realized,
                    err_pct
                ),
            ));
        }
    }

    // 6. Per-peripheral PWM timing reachability (TIM frequency_hz /
    // duty_resolution_bits). Mirrors `codegen::tim_timing`'s `timer_clk`
    // approximation (`[device].clock_hz`, not the resolved APB timer rate)
    // exactly, so a config that passes here never hits the PSC underflow
    // that function would otherwise saturate to 0 silently.
    for (instance, table) in &config.peripherals {
        let name = crate::model::peripheral_name(instance);
        if !name.starts_with("TIM") {
            continue;
        }
        let Some(freq) = table
            .0
            .get("frequency_hz")
            .and_then(toml::Value::as_integer)
        else {
            continue;
        };
        if freq <= 0 {
            continue;
        }
        let bits = table
            .0
            .get("duty_resolution_bits")
            .and_then(toml::Value::as_integer)
            .unwrap_or(16)
            .clamp(1, 31) as u32;
        let arr_plus_one: u64 = 1u64 << bits;
        let timer_clk = config.device.clock_hz.unwrap_or(180_000_000).max(1);
        // `freq` comes straight from TOML as an i64 and can be arbitrarily
        // large (e.g. a typo'd extra digit) — `checked_mul` avoids a debug-build
        // overflow panic, and an overflowing product is itself proof the
        // frequency is unreachable, so it's `> timer_clk` regardless.
        let unreachable = match (freq as u64).checked_mul(arr_plus_one) {
            Some(divisor) => divisor > timer_clk,
            None => true,
        };
        if unreachable {
            let actual_hz = timer_clk as f64 / arr_plus_one as f64;
            out.push(cc(
                &name,
                format!(
                    "{freq} Hz at {bits}-bit duty resolution is unachievable: a {} MHz timer clock divided by {arr_plus_one} (2^{bits} from duty_resolution_bits) yields PSC < 1, so the generated PWM would silently run at ~{actual_hz:.3} Hz instead of {freq} Hz — lower duty_resolution_bits or raise frequency_hz",
                    mhz(timer_clk as u32),
                ),
            ));
        }
    }

    out
}

/// If `baud` is NOT reachable from `pclk` within 2% (16× oversampling, USARTDIV
/// in `1..=0xFFFF`), return the closest realizable baud. `None` means reachable.
fn unreachable_baud(pclk: u32, baud: u32) -> Option<u32> {
    if baud == 0 {
        return None;
    }
    // Rounded divider; clamp into the hardware range.
    let mut div = (pclk + baud / 2) / baud;
    if div == 0 {
        return Some(0); // pclk < baud/2: utterly unreachable.
    }
    if div > 0xFFFF {
        div = 0xFFFF;
    }
    let realized = pclk / div;
    let diff = realized.abs_diff(baud);
    // 2% tolerance, integer form: diff/baud <= 1/50  <=>  diff*50 <= baud.
    if (diff as u64) * 50 <= baud as u64 {
        None
    } else {
        Some(realized)
    }
}

/// A human-readable percent error like `"3.5%"` for the conflict message.
fn pct_error(realized: u32, baud: u32) -> String {
    let diff = realized.abs_diff(baud) as u64;
    let tenths = diff * 1000 / baud as u64; // tenths of a percent
    format!("{}.{}%", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn f446() -> ClockTree {
        ClockTree::f446re()
    }
    fn f411() -> ClockTree {
        ClockTree::f411re()
    }

    fn validate_toml(text: &str, tree: &ClockTree) -> Vec<Conflict> {
        let cfg = config::parse(text).unwrap();
        validate(&cfg, tree)
    }

    fn resolve_toml(text: &str, tree: &ClockTree) -> ResolvedClocks {
        let cfg = config::parse(text).unwrap();
        resolve(&cfg.clocks, tree).unwrap()
    }

    #[test]
    fn f446_default_config_resolves_to_180mhz() {
        let r = resolve_toml("", &f446());
        assert_eq!(r.sysclk_hz, 180_000_000);
        assert_eq!(r.hclk_hz, 180_000_000);
        assert_eq!(r.pclk1_hz, 45_000_000); // /4
        assert_eq!(r.pclk2_hz, 90_000_000); // /2
        assert_eq!(r.apb1_timer_hz, 90_000_000); // doubled (presc != 1)
        assert_eq!(r.apb2_timer_hz, 180_000_000); // doubled
    }

    #[test]
    fn f411_default_config_resolves_to_100mhz() {
        let r = resolve_toml("", &f411());
        assert_eq!(r.sysclk_hz, 100_000_000);
        assert_eq!(r.hclk_hz, 100_000_000);
        assert_eq!(r.pclk1_hz, 50_000_000); // /2
        assert_eq!(r.pclk2_hz, 100_000_000); // /1
        assert_eq!(r.apb1_timer_hz, 100_000_000); // doubled (presc 2)
        assert_eq!(r.apb2_timer_hz, 100_000_000); // presc 1 -> not doubled
    }

    #[test]
    fn explicit_pll_math_matches_reference() {
        // HSE 8 MHz, M=8 (VCO_in 1 MHz), N=336 (VCO_out 336 MHz), P=4 -> 84 MHz.
        let r = resolve_toml(
            "[clocks]\nsource=\"pll\"\npll_source=\"hse\"\npll_m=8\npll_n=336\npll_p=4\n",
            &f446(),
        );
        assert_eq!(r.pll_vco_in_hz, 1_000_000);
        assert_eq!(r.pll_vco_out_hz, 336_000_000);
        assert_eq!(r.sysclk_hz, 84_000_000);
    }

    #[test]
    fn peripheral_freqs_pclk_vs_timer() {
        let tree = f446();
        let r = resolve_toml("", &tree);
        // USART2 on APB1 -> plain PCLK1.
        assert_eq!(peripheral_freq("USART2", &tree, &r).unwrap().hz, 45_000_000);
        // TIM2 on APB1 -> the doubled timer clock.
        assert_eq!(peripheral_freq("TIM2", &tree, &r).unwrap().hz, 90_000_000);
        // Unmodeled -> None.
        assert!(peripheral_freq("MADEUP", &tree, &r).is_none());
    }

    #[test]
    fn default_config_has_no_clock_conflicts() {
        assert_eq!(validate_toml("", &f446()), vec![]);
        assert_eq!(validate_toml("", &f411()), vec![]);
    }

    #[test]
    fn overclocked_apb1_is_a_clock_constraint() {
        // M1 EXIT (a): 180 MHz HCLK with APB1 prescaler 1 -> PCLK1 180 > 45 MHz.
        let conflicts = validate_toml(
            "[clocks]\npll_m=8\npll_n=360\npll_p=2\napb1_prescaler=1\n",
            &f446(),
        );
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        match &conflicts[0] {
            Conflict::ClockConstraint { node, reason } => {
                assert_eq!(node, "APB1");
                assert!(
                    reason.contains("exceeds") && reason.contains("45 MHz"),
                    "{reason}"
                );
            }
            other => panic!("expected ClockConstraint, got {other:?}"),
        }
    }

    #[test]
    fn overclocked_sysclk_via_pll() {
        // N too large: SYSCLK > 180 MHz on the F446.
        let conflicts = validate_toml("[clocks]\npll_m=8\npll_n=400\npll_p=2\n", &f446());
        assert!(
            conflicts
                .iter()
                .any(|c| matches!(c, Conflict::ClockConstraint { node, .. } if node == "SYSCLK")),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn pll_vco_in_too_low() {
        // M=64 -> VCO_in 8 MHz / 64 = 0.125 MHz < 1 MHz. (M=64 also out of the
        // 2..=63 range, so the structural check fires first — assert PLL node.)
        let conflicts = validate_toml("[clocks]\npll_m=64\npll_n=200\npll_p=2\n", &f446());
        assert!(
            conflicts
                .iter()
                .any(|c| matches!(c, Conflict::ClockConstraint { node, .. } if node == "PLL")),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn pll_vco_out_too_high() {
        // M=2 (VCO_in 4 MHz), N=200 -> VCO_out 800 MHz > 432 MHz.
        let conflicts = validate_toml("[clocks]\npll_m=2\npll_n=200\npll_p=2\n", &f446());
        assert!(
            conflicts.iter().any(|c| matches!(
                c,
                Conflict::ClockConstraint { node, reason }
                    if node == "PLL" && reason.contains("VCO output")
            )),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn invalid_pll_p_not_in_set() {
        let conflicts = validate_toml("[clocks]\npll_m=8\npll_n=180\npll_p=3\n", &f446());
        assert!(
            conflicts.iter().any(|c| matches!(
                c,
                Conflict::ClockConstraint { node, reason }
                    if node == "PLL" && reason.contains("allowed dividers")
            )),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn invalid_apb_prescaler() {
        let conflicts = validate_toml("[clocks]\napb1_prescaler=32\n", &f446());
        assert!(
            conflicts.iter().any(|c| matches!(
                c,
                Conflict::ClockConstraint { node, reason }
                    if node == "APB1" && reason.contains("allowed values")
            )),
            "got {conflicts:?}"
        );
    }

    #[test]
    fn unknown_sysclk_source_string_no_cascade() {
        let conflicts = validate_toml("[clocks]\nsource=\"turbo\"\n", &f446());
        assert_eq!(conflicts.len(), 1);
        assert!(matches!(
            &conflicts[0],
            Conflict::ClockConstraint { node, reason }
                if node == "SYSCLK" && reason.contains("unknown")
        ));
    }

    #[test]
    fn baud_unreachable_from_low_pclk() {
        // M1 EXIT (b): HSI SYSCLK 16 MHz -> PCLK1 16 MHz; a 20 Mbaud target is
        // unreachable (nearest divider gives 16 Mbaud, 20% error).
        let conflicts = validate_toml(
            "[clocks]\nsource=\"hsi\"\n\n[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nbaud=20000000\n",
            &f446(),
        );
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(
            &conflicts[0],
            Conflict::ClockConstraint { node, reason }
                if node == "USART2" && reason.contains("20000000 baud") && reason.contains("unreachable")
        ));
    }

    #[test]
    fn reachable_baud_is_clean() {
        // 115200 from the default 45 MHz PCLK1 is reachable within 2%.
        let conflicts = validate_toml(
            "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nbaud=115200\n",
            &f446(),
        );
        assert_eq!(conflicts, vec![]);
    }

    #[test]
    fn pwm_timing_unachievable_is_a_clock_constraint() {
        // Issue #29: 1 Hz at 31-bit duty resolution needs a divisor of 2^31,
        // which exceeds the default 180 MHz timer clock — codegen's
        // `tim_timing` would otherwise saturate PSC to 0 with no diagnostic.
        let conflicts = validate_toml(
            "[peripherals.tim2]\nchannel1=\"PA0\"\nfrequency_hz=1\nduty_resolution_bits=31\n",
            &f446(),
        );
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(
            &conflicts[0],
            Conflict::ClockConstraint { node, reason }
                if node == "TIM2" && reason.contains("unachievable") && reason.contains("1 Hz")
        ));
    }

    #[test]
    fn reachable_pwm_timing_is_clean() {
        // 1 kHz at 16-bit duty resolution from the default 180 MHz timer
        // clock is comfortably achievable.
        let conflicts = validate_toml(
            "[peripherals.tim2]\nchannel1=\"PA0\"\nfrequency_hz=1000\nduty_resolution_bits=16\n",
            &f446(),
        );
        assert_eq!(conflicts, vec![]);
    }

    #[test]
    fn huge_pwm_frequency_is_a_clock_constraint_not_a_panic() {
        // Issue #50: `(freq as u64) * arr_plus_one` overflowed u64 silently
        // for a large TOML i64 frequency_hz (a debug-build panic, or a
        // wrapped-around-small product that wrongly passed the `> timer_clk`
        // check in release). A frequency this large is unreachable no matter
        // how it's computed, so it must surface a diagnostic either way.
        let conflicts = validate_toml(
            "[peripherals.tim2]\nchannel1=\"PA0\"\nfrequency_hz=9000000000000000000\nduty_resolution_bits=16\n",
            &f446(),
        );
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(
            &conflicts[0],
            Conflict::ClockConstraint { node, reason }
                if node == "TIM2" && reason.contains("unachievable")
        ));
    }

    #[test]
    fn no_hse_modeled_is_a_clock_constraint_not_a_panic() {
        // Issue #44: default_effective() divided max_sysclk_hz * pll_p by
        // vco_in (derived from HSE) with no zero guard. A family with no HSE
        // modeled — or any future family whose default PLL math bottoms out
        // at 0 Hz — must surface a diagnostic, not panic.
        let tree = ClockTree::without_oscillator(f446(), Oscillator::Hse);
        let conflicts = validate_toml("", &tree);
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        assert!(matches!(
            &conflicts[0],
            Conflict::ClockConstraint { node, reason }
                if node == "PLL" && reason.contains("must not be zero")
        ));
        assert_eq!(resolve_toml_opt("", &tree), None);
    }

    fn resolve_toml_opt(text: &str, tree: &ClockTree) -> Option<ResolvedClocks> {
        let cfg = config::parse(text).unwrap();
        resolve(&cfg.clocks, tree)
    }

    #[test]
    fn mhz_formats_fractions() {
        assert_eq!(mhz(45_000_000), "45");
        assert_eq!(mhz(22_500_000), "22.5");
        assert_eq!(mhz(500_000), "0.5");
    }
}
