//! The IRQ/NVIC **data model** for the STM32F4 families Nucleus supports.
//!
//! This is the static silicon description the v2 IRQ verifier (M3) reasons
//! over. It is **pure data**: which NVIC vector services a given EXTI line,
//! and which NVIC vector(s) service a given peripheral's interrupt. It
//! performs **no conflict detection** — finding two EXTI lines sharing one
//! vector, or a peripheral interrupt enabled in `stm32.toml` with no handler,
//! is `nucleus-compiler`'s job (the M3 solver consumes this model), mirroring
//! how [`crate::dma`] is pure data and the M2 solver does the arbitration.
//!
//! The vendored `packdata/` XML carries **no** NVIC/IRQ information (only
//! pin↔AF mux data), so unlike [`crate::Database`] this model cannot be
//! generated from a pack source — it is necessarily hand-maintained, and is
//! cross-validated by a mandatory reference-manual seed test (see `tests`).
//! Values are cited to ST's reference manuals: **RM0390** (F446, §10.1.2 /
//! Table 38, the interrupt and exception vectors) and **RM0383** (F411,
//! §10.1.2, the equivalent vector table). The EXTI→NVIC grouping (lines 0–4
//! individually vectored, 5–9 sharing `EXTI9_5`, 10–15 sharing `EXTI15_10`)
//! is identical on both families — same NVIC layout, same vector names, in
//! both manuals.
//!
//! Peripheral coverage matches exactly the peripheral kinds
//! `nucleus_compiler::model::roles_for` models (USART/UART, SPI, I2C, TIM),
//! restricted to the instances each family actually has (see
//! [`crate::dma`] and [`crate::clock`] for the same F446-vs-F411 split:
//! the F411 omits USART3/UART4/UART5, both families have USART1/2/6,
//! SPI1–4, I2C1–3, and TIM2–5). Never guess a vector name: I2Cx interrupts
//! are unusual in having **two** vectors per instance (event and error),
//! everything else modeled here has exactly one.

/// Map an EXTI line number (0..=15) to the NVIC vector name that services it.
///
/// Lines 0–4 each have a dedicated vector (`EXTI0`..`EXTI4`); lines 5–9 share
/// `EXTI9_5`; lines 10–15 share `EXTI15_10`. This grouping is identical on
/// the F446 and F411 (RM0390 / RM0383 Table 38 vector table).
pub const fn group_for(line: u8) -> &'static str {
    match line {
        0 => "EXTI0",
        1 => "EXTI1",
        2 => "EXTI2",
        3 => "EXTI3",
        4 => "EXTI4",
        5..=9 => "EXTI9_5",
        10..=15 => "EXTI15_10",
        _ => "INVALID_EXTI_LINE",
    }
}

/// EXTI line → NVIC vector groupings for one device family.
///
/// Both supported families share the same EXTI/NVIC layout, so this struct
/// is a thin, explicit wrapper around [`group_for`] rather than a per-family
/// table — kept as its own type (mirroring [`IrqMap`]'s family-parameterized
/// shape) so callers don't need to special-case "EXTI is the same everywhere".
#[derive(Debug, Clone, Copy)]
pub struct ExtiGroups;

impl ExtiGroups {
    /// The EXTI groupings for the STM32F446RE (RM0390 Table 38).
    pub const fn f446re() -> ExtiGroups {
        ExtiGroups
    }

    /// The EXTI groupings for the STM32F411RE (RM0383 Table 38-equivalent).
    pub const fn f411re() -> ExtiGroups {
        ExtiGroups
    }

    /// The NVIC vector name servicing EXTI `line` (0..=15).
    pub const fn group_for(&self, line: u8) -> &'static str {
        group_for(line)
    }
}

/// One row of the reference-manual vector table: a peripheral and the NVIC
/// vector name(s) that service its interrupt(s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeripheralIrq {
    /// Database peripheral name, e.g. `"USART2"`.
    pub peripheral: &'static str,
    /// The NVIC vector name(s) for this peripheral, in vector-table order.
    /// Most peripherals have exactly one; I2Cx has two (`I2Cx_EV`, `I2Cx_ER`).
    pub vectors: &'static [&'static str],
}

/// The complete peripheral-interrupt vector map for one device family.
#[derive(Debug, Clone, Copy)]
pub struct IrqMap {
    rows: &'static [PeripheralIrq],
}

impl IrqMap {
    /// The IRQ vector map for the STM32F446RE (RM0390 Table 38).
    pub const fn f446re() -> IrqMap {
        IrqMap { rows: data::F446RE }
    }

    /// The IRQ vector map for the STM32F411RE (RM0383 vector table).
    pub const fn f411re() -> IrqMap {
        IrqMap { rows: data::F411RE }
    }

    /// Every modeled row for this family.
    pub const fn rows(&self) -> &'static [PeripheralIrq] {
        self.rows
    }

    /// Whether this family models an interrupt vector for `peripheral`.
    pub fn has_peripheral(&self, peripheral: &str) -> bool {
        self.rows.iter().any(|r| r.peripheral == peripheral)
    }

    /// The NVIC vector name(s) servicing `peripheral`'s interrupt(s), or an
    /// empty slice if the family does not model this peripheral (the solver
    /// then skips it rather than guessing — never a false error, matching
    /// the `peripheral_bus` discipline in `nucleus_compiler::model`).
    pub fn vectors(&self, peripheral: &str) -> &'static [&'static str] {
        match self.rows.iter().find(|r| r.peripheral == peripheral) {
            Some(r) => r.vectors,
            None => &[],
        }
    }
}

mod data {
    use super::PeripheralIrq;

    const fn row(peripheral: &'static str, vectors: &'static [&'static str]) -> PeripheralIrq {
        PeripheralIrq {
            peripheral,
            vectors,
        }
    }

    // --- STM32F446RE (RM0390 §10.1.2 Table 38, interrupt vector table) -------
    //
    // One row per peripheral kind `nucleus_compiler::model::roles_for` models
    // (USART/UART, SPI, I2C, TIM), restricted to instances the F446 has.
    // Each vector name is the exact NVIC IRQ handler name from the table.
    pub(super) const F446RE: &[PeripheralIrq] = &[
        row("USART1", &["USART1"]),
        row("USART2", &["USART2"]),
        row("USART3", &["USART3"]),
        row("UART4", &["UART4"]),
        row("UART5", &["UART5"]),
        row("USART6", &["USART6"]),
        row("SPI1", &["SPI1"]),
        row("SPI2", &["SPI2"]),
        row("SPI3", &["SPI3"]),
        row("SPI4", &["SPI4"]),
        // I2Cx has two vectors: event (transfer progress) and error (bus
        // errors / NACK / arbitration loss) — RM0390 Table 38 lists both.
        row("I2C1", &["I2C1_EV", "I2C1_ER"]),
        row("I2C2", &["I2C2_EV", "I2C2_ER"]),
        row("I2C3", &["I2C3_EV", "I2C3_ER"]),
        row("TIM2", &["TIM2"]),
        row("TIM3", &["TIM3"]),
        row("TIM4", &["TIM4"]),
        row("TIM5", &["TIM5"]),
    ];

    // --- STM32F411RE (RM0383 §10.1.2 vector table) ---------------------------
    //
    // The F411 is a smaller package: no UART4/5, no USART3 (mirrors the same
    // omission in `dma.rs` and `clock.rs`'s F411 tables). The shared
    // peripherals (USART1/2/6, SPI1-4, I2C1-3, TIM2-5) keep the same vector
    // names as the F446 — the vector table layout is identical for the parts
    // both families share.
    pub(super) const F411RE: &[PeripheralIrq] = &[
        row("USART1", &["USART1"]),
        row("USART2", &["USART2"]),
        row("USART6", &["USART6"]),
        row("SPI1", &["SPI1"]),
        row("SPI2", &["SPI2"]),
        row("SPI3", &["SPI3"]),
        row("SPI4", &["SPI4"]),
        row("I2C1", &["I2C1_EV", "I2C1_ER"]),
        row("I2C2", &["I2C2_EV", "I2C2_ER"]),
        row("I2C3", &["I2C3_EV", "I2C3_ER"]),
        row("TIM2", &["TIM2"]),
        row("TIM3", &["TIM3"]),
        row("TIM4", &["TIM4"]),
        row("TIM5", &["TIM5"]),
    ];
}
