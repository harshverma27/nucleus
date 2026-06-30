//! The clock-tree **data model** for the STM32F4 families Nucleus supports.
//!
//! This is the static silicon description the v2 clock-tree solver reasons over.
//! It is **pure data**: oscillator frequencies, the main PLL divider ranges and
//! VCO constraints, the SYSCLK source set, the AHB/APB prescaler sets, the
//! per-peripheral clock derivation, and the per-family bus frequency limits.
//! It performs **no arithmetic and no validation** — computing each peripheral's
//! actual frequency and rejecting illegal configurations is `nucleus-compiler`'s
//! job (the M1 solver consumes this model).
//!
//! The vendored `packdata/` XML carries **no** clock-tree information (only pin↔AF
//! mux data), so unlike [`crate::Database`] this model cannot be generated from a
//! pack source — it is necessarily hand-maintained, and is cross-validated by a
//! mandatory reference-manual seed test (see `tests`). Values are cited to ST's
//! reference manuals: **RM0390** (F446) and **RM0383** (F411).

/// A closed inclusive frequency range in hertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreqRange {
    pub min_hz: u32,
    pub max_hz: u32,
}

impl FreqRange {
    pub const fn new(min_hz: u32, max_hz: u32) -> FreqRange {
        FreqRange { min_hz, max_hz }
    }

    /// Whether `hz` lies within `[min_hz, max_hz]`.
    pub const fn contains(self, hz: u32) -> bool {
        hz >= self.min_hz && hz <= self.max_hz
    }
}

/// A closed inclusive integer divider/multiplier range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DividerRange {
    pub min: u32,
    pub max: u32,
}

impl DividerRange {
    pub const fn new(min: u32, max: u32) -> DividerRange {
        DividerRange { min, max }
    }

    /// Whether `v` lies within `[min, max]`.
    pub const fn contains(self, v: u32) -> bool {
        v >= self.min && v <= self.max
    }
}

/// A clock oscillator source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oscillator {
    /// High-speed internal RC.
    Hsi,
    /// High-speed external (crystal / ST-LINK MCO on the NUCLEO boards).
    Hse,
    /// Low-speed internal RC.
    Lsi,
    /// Low-speed external (32.768 kHz crystal).
    Lse,
}

/// One oscillator's nominal frequency and the accepted input range.
///
/// For the internal RC oscillators (`Hsi`/`Lsi`) the nominal is fixed and the
/// range collapses to that single value. For `Hse`/`Lse` the nominal is the
/// frequency present on the NUCLEO boards and the range is the silicon-accepted
/// crystal range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OscillatorSpec {
    pub source: Oscillator,
    pub nominal_hz: u32,
    pub range: FreqRange,
}

/// The main PLL's divider ranges and VCO constraints.
///
/// On the STM32F4 the main PLL computes
/// `VCO_in = source / M`, `VCO_out = VCO_in * N`, `PLLCLK = VCO_out / P`,
/// with an independent `Q` divider feeding the 48 MHz domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PllSpec {
    /// `M` input divider range.
    pub m: DividerRange,
    /// `N` VCO multiplier range.
    pub n: DividerRange,
    /// `P` system-clock output divider — a fixed small set, not a range.
    pub p: &'static [u32],
    /// `Q` 48 MHz-domain output divider range.
    pub q: DividerRange,
    /// Accepted PLL input frequency (`VCO_in`, after the `M` divider).
    pub vco_in: FreqRange,
    /// Accepted VCO output frequency (`VCO_out`).
    pub vco_out: FreqRange,
    /// Selectable PLL clock sources.
    pub sources: &'static [Oscillator],
}

/// A device bus clock domain.
///
/// Mirrors `nucleus_compiler::model::Bus` so peripheral→bus assignments line up
/// across the two crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Ahb1,
    Apb1,
    Apb2,
}

impl Bus {
    pub const fn name(self) -> &'static str {
        match self {
            Bus::Ahb1 => "AHB1",
            Bus::Apb1 => "APB1",
            Bus::Apb2 => "APB2",
        }
    }
}

/// A selectable SYSCLK source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysclkSource {
    Hsi,
    Hse,
    Pll,
}

/// The per-family maximum frequencies (in hertz) the silicon permits. These are
/// the limits the solver validates resolved frequencies against, and they are
/// the headline difference between the two families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SiliconLimits {
    pub max_sysclk_hz: u32,
    pub max_ahb_hz: u32,
    pub max_apb1_hz: u32,
    pub max_apb2_hz: u32,
}

/// Which bus clock domain feeds a peripheral instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeripheralClock {
    /// Database peripheral name, e.g. `"USART2"`.
    pub peripheral: &'static str,
    pub bus: Bus,
}

/// The complete clock-tree model for one device family.
#[derive(Debug, Clone, Copy)]
pub struct ClockTree {
    oscillators: &'static [OscillatorSpec],
    pll: PllSpec,
    sysclk_sources: &'static [SysclkSource],
    ahb_prescalers: &'static [u32],
    apb1_prescalers: &'static [u32],
    apb2_prescalers: &'static [u32],
    limits: SiliconLimits,
    peripheral_clocks: &'static [PeripheralClock],
}

impl ClockTree {
    /// The clock tree for the STM32F446RE (RM0390).
    pub const fn f446re() -> ClockTree {
        data::F446RE
    }

    /// The clock tree for the STM32F411RE (RM0383).
    pub const fn f411re() -> ClockTree {
        data::F411RE
    }

    pub const fn oscillators(&self) -> &'static [OscillatorSpec] {
        self.oscillators
    }

    /// The spec for a single oscillator `source`, if the family exposes it.
    pub fn oscillator(&self, source: Oscillator) -> Option<&OscillatorSpec> {
        self.oscillators.iter().find(|o| o.source == source)
    }

    pub const fn pll(&self) -> PllSpec {
        self.pll
    }

    pub const fn sysclk_sources(&self) -> &'static [SysclkSource] {
        self.sysclk_sources
    }

    pub const fn ahb_prescalers(&self) -> &'static [u32] {
        self.ahb_prescalers
    }

    /// The legal prescaler set for an APB bus, or the AHB set for `Bus::Ahb1`.
    pub const fn prescalers(&self, bus: Bus) -> &'static [u32] {
        match bus {
            Bus::Ahb1 => self.ahb_prescalers,
            Bus::Apb1 => self.apb1_prescalers,
            Bus::Apb2 => self.apb2_prescalers,
        }
    }

    pub const fn limits(&self) -> SiliconLimits {
        self.limits
    }

    pub const fn peripheral_clocks(&self) -> &'static [PeripheralClock] {
        self.peripheral_clocks
    }

    /// The bus that feeds `peripheral`, or `None` if the family does not model it
    /// (the solver then skips the clock check rather than guessing — never a
    /// false error, matching the v1 `peripheral_bus` discipline).
    pub fn peripheral_bus(&self, peripheral: &str) -> Option<Bus> {
        self.peripheral_clocks
            .iter()
            .find(|p| p.peripheral == peripheral)
            .map(|p| p.bus)
    }

    /// The APBx-timer clock multiplier: timers on an APB bus run at the APB clock
    /// when the APB prescaler is 1, and at **twice** the APB clock otherwise
    /// (RM0390 §6.2 / RM0383 §6.2, "APBx timer clocks"). Returns `1` for the AHB
    /// bus (no doubling rule applies).
    pub const fn timer_multiplier(bus: Bus, apb_prescaler: u32) -> u32 {
        match bus {
            Bus::Apb1 | Bus::Apb2 if apb_prescaler != 1 => 2,
            _ => 1,
        }
    }

    /// `base` with `source` removed from the modeled oscillator set, as if a
    /// family's pack data never described it. Lets callers (the M1 clock
    /// solver) exercise the "this family's model has a gap" path on demand,
    /// since the two hand-seeded families both model all four oscillators —
    /// not meant for production family data.
    pub fn without_oscillator(base: ClockTree, source: Oscillator) -> ClockTree {
        let kept: Vec<OscillatorSpec> = base
            .oscillators
            .iter()
            .copied()
            .filter(|o| o.source != source)
            .collect();
        ClockTree {
            oscillators: Box::leak(kept.into_boxed_slice()),
            ..base
        }
    }
}

mod data {
    use super::{
        Bus, ClockTree, DividerRange, FreqRange, Oscillator, OscillatorSpec, PeripheralClock,
        PllSpec, SiliconLimits, SysclkSource,
    };

    // Oscillators are identical across both families on the NUCLEO boards:
    // 16 MHz HSI, 32 kHz LSI, 32.768 kHz LSE, and an 8 MHz HSE delivered by the
    // ST-LINK MCO (HSE crystal range per the datasheet is 4–26 MHz).
    const OSCILLATORS: &[OscillatorSpec] = &[
        OscillatorSpec {
            source: Oscillator::Hsi,
            nominal_hz: 16_000_000,
            range: FreqRange::new(16_000_000, 16_000_000),
        },
        OscillatorSpec {
            source: Oscillator::Hse,
            nominal_hz: 8_000_000,
            range: FreqRange::new(4_000_000, 26_000_000),
        },
        OscillatorSpec {
            source: Oscillator::Lsi,
            nominal_hz: 32_000,
            range: FreqRange::new(32_000, 32_000),
        },
        OscillatorSpec {
            source: Oscillator::Lse,
            nominal_hz: 32_768,
            range: FreqRange::new(32_768, 32_768),
        },
    ];

    // RCC prescaler value sets (RM0390 §6.3.3 / RM0383 §6.3.3, RCC_CFGR HPRE/PPREx).
    const AHB_PRESCALERS: &[u32] = &[1, 2, 4, 8, 16, 64, 128, 256, 512];
    const APB_PRESCALERS: &[u32] = &[1, 2, 4, 8, 16];

    const PLL_SOURCES: &[Oscillator] = &[Oscillator::Hsi, Oscillator::Hse];
    const PLL_P: &[u32] = &[2, 4, 6, 8];

    // --- STM32F446RE (RM0390) ------------------------------------------------

    // RM0390 §6.3.2 (RCC_PLLCFGR): M 2..=63, N 50..=432, P {2,4,6,8}, Q 2..=15;
    // VCO input 1..=2 MHz, VCO output 100..=432 MHz.
    const F446_PLL: PllSpec = PllSpec {
        m: DividerRange::new(2, 63),
        n: DividerRange::new(50, 432),
        p: PLL_P,
        q: DividerRange::new(2, 15),
        vco_in: FreqRange::new(1_000_000, 2_000_000),
        vco_out: FreqRange::new(100_000_000, 432_000_000),
        sources: PLL_SOURCES,
    };

    // RM0390 §6.2: SYSCLK ≤ 180 MHz, HCLK ≤ 180 MHz, PCLK1 ≤ 45 MHz, PCLK2 ≤ 90 MHz.
    const F446_LIMITS: SiliconLimits = SiliconLimits {
        max_sysclk_hz: 180_000_000,
        max_ahb_hz: 180_000_000,
        max_apb1_hz: 45_000_000,
        max_apb2_hz: 90_000_000,
    };

    pub(super) const F446RE: ClockTree = ClockTree {
        oscillators: OSCILLATORS,
        pll: F446_PLL,
        sysclk_sources: &[SysclkSource::Hsi, SysclkSource::Hse, SysclkSource::Pll],
        ahb_prescalers: AHB_PRESCALERS,
        apb1_prescalers: APB_PRESCALERS,
        apb2_prescalers: APB_PRESCALERS,
        limits: F446_LIMITS,
        peripheral_clocks: F446_PERIPHERAL_CLOCKS,
    };

    // Bus assignments per RM0390 RCC peripheral-clock-enable registers; mirrors
    // `nucleus_compiler::model::peripheral_bus`.
    const F446_PERIPHERAL_CLOCKS: &[PeripheralClock] = &[
        // APB2
        pc("USART1", Bus::Apb2),
        pc("USART6", Bus::Apb2),
        pc("SPI1", Bus::Apb2),
        pc("SPI4", Bus::Apb2),
        pc("SDIO", Bus::Apb2),
        pc("ADC1", Bus::Apb2),
        pc("ADC2", Bus::Apb2),
        pc("ADC3", Bus::Apb2),
        pc("SAI1", Bus::Apb2),
        pc("SAI2", Bus::Apb2),
        pc("TIM1", Bus::Apb2),
        pc("TIM8", Bus::Apb2),
        pc("TIM9", Bus::Apb2),
        pc("TIM10", Bus::Apb2),
        pc("TIM11", Bus::Apb2),
        // APB1
        pc("USART2", Bus::Apb1),
        pc("USART3", Bus::Apb1),
        pc("UART4", Bus::Apb1),
        pc("UART5", Bus::Apb1),
        pc("SPI2", Bus::Apb1),
        pc("SPI3", Bus::Apb1),
        pc("I2C1", Bus::Apb1),
        pc("I2C2", Bus::Apb1),
        pc("I2C3", Bus::Apb1),
        pc("FMPI2C1", Bus::Apb1),
        pc("CAN1", Bus::Apb1),
        pc("CAN2", Bus::Apb1),
        pc("SPDIFRX", Bus::Apb1),
        pc("CEC", Bus::Apb1),
        pc("TIM2", Bus::Apb1),
        pc("TIM3", Bus::Apb1),
        pc("TIM4", Bus::Apb1),
        pc("TIM5", Bus::Apb1),
        pc("TIM6", Bus::Apb1),
        pc("TIM7", Bus::Apb1),
        pc("TIM12", Bus::Apb1),
        pc("TIM13", Bus::Apb1),
        pc("TIM14", Bus::Apb1),
        // AHB1
        pc("DCMI", Bus::Ahb1),
        pc("QUADSPI", Bus::Ahb1),
        pc("FMC", Bus::Ahb1),
    ];

    // --- STM32F411RE (RM0383) ------------------------------------------------

    // RM0383 §6.3.2 (RCC_PLLCFGR): M 2..=63, N 50..=432, P {2,4,6,8}, Q 2..=15;
    // VCO input 1..=2 MHz, VCO output 100..=432 MHz.
    const F411_PLL: PllSpec = PllSpec {
        m: DividerRange::new(2, 63),
        n: DividerRange::new(50, 432),
        p: PLL_P,
        q: DividerRange::new(2, 15),
        vco_in: FreqRange::new(1_000_000, 2_000_000),
        vco_out: FreqRange::new(100_000_000, 432_000_000),
        sources: PLL_SOURCES,
    };

    // RM0383 §6.2: SYSCLK ≤ 100 MHz, HCLK ≤ 100 MHz, PCLK1 ≤ 50 MHz, PCLK2 ≤ 100 MHz.
    // These limits are the deliberate family difference from the F446.
    const F411_LIMITS: SiliconLimits = SiliconLimits {
        max_sysclk_hz: 100_000_000,
        max_ahb_hz: 100_000_000,
        max_apb1_hz: 50_000_000,
        max_apb2_hz: 100_000_000,
    };

    pub(super) const F411RE: ClockTree = ClockTree {
        oscillators: OSCILLATORS,
        pll: F411_PLL,
        sysclk_sources: &[SysclkSource::Hsi, SysclkSource::Hse, SysclkSource::Pll],
        ahb_prescalers: AHB_PRESCALERS,
        apb1_prescalers: APB_PRESCALERS,
        apb2_prescalers: APB_PRESCALERS,
        limits: F411_LIMITS,
        peripheral_clocks: F411_PERIPHERAL_CLOCKS,
    };

    // The F411 is a smaller package: no UART4/5, no USART3, no CAN, no SAI2,
    // no third ADC, etc. (RM0383 RCC enable registers).
    const F411_PERIPHERAL_CLOCKS: &[PeripheralClock] = &[
        // APB2
        pc("USART1", Bus::Apb2),
        pc("USART6", Bus::Apb2),
        pc("SPI1", Bus::Apb2),
        pc("SPI4", Bus::Apb2),
        pc("SPI5", Bus::Apb2),
        pc("SDIO", Bus::Apb2),
        pc("ADC1", Bus::Apb2),
        pc("TIM1", Bus::Apb2),
        pc("TIM9", Bus::Apb2),
        pc("TIM10", Bus::Apb2),
        pc("TIM11", Bus::Apb2),
        // APB1
        pc("USART2", Bus::Apb1),
        pc("SPI2", Bus::Apb1),
        pc("SPI3", Bus::Apb1),
        pc("I2C1", Bus::Apb1),
        pc("I2C2", Bus::Apb1),
        pc("I2C3", Bus::Apb1),
        pc("TIM2", Bus::Apb1),
        pc("TIM3", Bus::Apb1),
        pc("TIM4", Bus::Apb1),
        pc("TIM5", Bus::Apb1),
    ];

    const fn pc(peripheral: &'static str, bus: Bus) -> PeripheralClock {
        PeripheralClock { peripheral, bus }
    }
}
