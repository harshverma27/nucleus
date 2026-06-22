//! The DMA controller **data model** for the STM32F4 families Nucleus supports.
//!
//! This is the static silicon description the v2 DMA-arbitration solver (M2)
//! reasons over. It is **pure data**: the two DMA controllers (DMA1/DMA2), their
//! eight streams each, the eight request channels per stream, and the
//! peripheral-request mapping — which `(peripheral, direction)` request is served
//! by which `(controller, stream, channel)` slots. It performs **no contention
//! detection and no arbitration** — finding two enabled peripherals fighting for
//! one stream and proposing a free alternative is `nucleus-compiler`'s job (the
//! M2 solver consumes this model).
//!
//! The vendored `packdata/` XML carries **no** DMA information (only pin↔AF mux
//! data), so unlike [`crate::Database`] this model cannot be generated from a
//! pack source — it is necessarily hand-maintained, and is cross-validated by a
//! mandatory reference-manual seed test (see `tests`). Values are cited to ST's
//! reference manuals: **RM0390** (F446, DMA1 Table 28 / DMA2 Table 29) and
//! **RM0383** (F411, DMA1 Table 27 / DMA2 Table 28).

/// Which of the two DMA controllers a stream lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Controller {
    Dma1,
    Dma2,
}

impl Controller {
    pub const fn name(self) -> &'static str {
        match self {
            Controller::Dma1 => "DMA1",
            Controller::Dma2 => "DMA2",
        }
    }
}

/// The transfer direction of a DMA request, as seen from the peripheral.
///
/// On the STM32F4 a peripheral's TX and RX paths are *independent* DMA requests
/// that generally map to different stream/channel slots, so direction is part of
/// the request key. SPI's `MOSI` is the TX path and `MISO` is the RX path; the
/// model exposes both spellings ([`Direction::from_signal`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// Memory → peripheral (e.g. USART_TX, SPI_TX/MOSI, I2C_TX, DAC).
    Tx,
    /// Peripheral → memory (e.g. USART_RX, SPI_RX/MISO, I2C_RX, ADC).
    Rx,
}

impl Direction {
    pub const fn name(self) -> &'static str {
        match self {
            Direction::Tx => "TX",
            Direction::Rx => "RX",
        }
    }

    /// Map a peripheral signal spelling to a DMA direction.
    ///
    /// Accepts the canonical `TX`/`RX` as well as SPI's `MOSI`/`MISO` aliases so
    /// the solver can resolve an `stm32.toml` `[peripherals.spi1] mosi = ...`
    /// line to its TX request slot.
    pub fn from_signal(signal: &str) -> Option<Direction> {
        match signal {
            "TX" | "MOSI" => Some(Direction::Tx),
            "RX" | "MISO" => Some(Direction::Rx),
            _ => None,
        }
    }
}

/// One concrete DMA slot: a `(controller, stream, channel)` triple.
///
/// `stream` is 0..=7 and `channel` is 0..=7 (RM0390 §9 / RM0383 §9, the
/// DMA_SxCR.CHSEL field selects one of eight channels per stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot {
    pub controller: Controller,
    pub stream: u8,
    pub channel: u8,
}

impl Slot {
    pub const fn new(controller: Controller, stream: u8, channel: u8) -> Slot {
        Slot {
            controller,
            stream,
            channel,
        }
    }
}

/// One row of the reference-manual DMA request-mapping table: a
/// `(peripheral, direction)` request and the slot that serves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRequest {
    /// Database peripheral name, e.g. `"USART2"`.
    pub peripheral: &'static str,
    pub direction: Direction,
    pub slot: Slot,
}

/// The complete DMA request-mapping model for one device family.
#[derive(Debug, Clone, Copy)]
pub struct DmaMap {
    requests: &'static [DmaRequest],
}

impl DmaMap {
    /// The DMA request map for the STM32F446RE (RM0390 Tables 28/29).
    pub const fn f446re() -> DmaMap {
        DmaMap {
            requests: data::F446RE,
        }
    }

    /// The DMA request map for the STM32F411RE (RM0383 Tables 27/28).
    pub const fn f411re() -> DmaMap {
        DmaMap {
            requests: data::F411RE,
        }
    }

    /// Every modeled request row for this family.
    pub const fn requests(&self) -> &'static [DmaRequest] {
        self.requests
    }

    /// Enumerate the candidate `(controller, stream, channel)` slots that can
    /// serve `(peripheral, direction)`, in deterministic table order.
    ///
    /// Most STM32F4 requests have exactly one slot, but some peripherals appear
    /// in several stream rows on the same channel (giving the solver a free
    /// alternative when the first is contended). Returns an empty vector if the
    /// family does not model the request (the solver then skips it rather than
    /// guessing — never a false error, matching the `peripheral_bus` discipline).
    pub fn candidates(&self, peripheral: &str, direction: Direction) -> Vec<Slot> {
        self.requests
            .iter()
            .filter(|r| r.peripheral == peripheral && r.direction == direction)
            .map(|r| r.slot)
            .collect()
    }

    /// Whether this family models any DMA request for `peripheral`.
    pub fn has_peripheral(&self, peripheral: &str) -> bool {
        self.requests.iter().any(|r| r.peripheral == peripheral)
    }
}

mod data {
    use super::{Controller::Dma1, Controller::Dma2, Direction::Rx, Direction::Tx};
    use super::{DmaRequest, Slot};

    const fn req(
        peripheral: &'static str,
        direction: super::Direction,
        controller: super::Controller,
        stream: u8,
        channel: u8,
    ) -> DmaRequest {
        DmaRequest {
            peripheral,
            direction,
            slot: Slot::new(controller, stream, channel),
        }
    }

    // --- STM32F446RE (RM0390 Table 28 = DMA1, Table 29 = DMA2) ---------------
    //
    // Each row is (peripheral, direction) -> (controller, stream, channel).
    // Stream/channel taken directly from the request-mapping tables. Where a
    // peripheral/direction appears in multiple stream rows of one table, every
    // row is listed (in stream order) so the solver has a free alternative.
    pub(super) const F446RE: &[DmaRequest] = &[
        // ---- DMA1 (RM0390 Table 28) ----
        // SPI3 (channel 0): RX stream 0/2, TX stream 5/7.
        req("SPI3", Rx, Dma1, 0, 0),
        req("SPI3", Rx, Dma1, 2, 0),
        req("SPI3", Tx, Dma1, 5, 0),
        req("SPI3", Tx, Dma1, 7, 0),
        // I2C1 (channel 1): RX stream 0/5, TX stream 6/7.
        req("I2C1", Rx, Dma1, 0, 1),
        req("I2C1", Rx, Dma1, 5, 1),
        req("I2C1", Tx, Dma1, 6, 1),
        req("I2C1", Tx, Dma1, 7, 1),
        // TIM4 (channel 2): CH1 s0, UP s6, CH2 s3, CH3 s7 — modeled TX/RX as UP.
        req("TIM4", Rx, Dma1, 0, 2),
        req("TIM4", Tx, Dma1, 6, 2),
        // I2C3 (channel 3): RX stream 1/2, TX stream 4.
        req("I2C3", Rx, Dma1, 1, 3),
        req("I2C3", Rx, Dma1, 2, 3),
        req("I2C3", Tx, Dma1, 4, 3),
        // UART5 (channel 4): RX stream 0, TX stream 7.
        req("UART5", Rx, Dma1, 0, 4),
        req("UART5", Tx, Dma1, 7, 4),
        // USART3 (channel 4): RX stream 1, TX stream 3.
        req("USART3", Rx, Dma1, 1, 4),
        req("USART3", Tx, Dma1, 3, 4),
        // UART4 (channel 4): RX stream 2, TX stream 4.
        req("UART4", Rx, Dma1, 2, 4),
        req("UART4", Tx, Dma1, 4, 4),
        // USART2 (channel 4): RX stream 5, TX stream 6.
        req("USART2", Rx, Dma1, 5, 4),
        req("USART2", Tx, Dma1, 6, 4),
        // TIM2 (channel 3): UP/CH3 s1, CH1 s5, CH2/CH4/UP s6, CH4/TRIG s7.
        req("TIM2", Rx, Dma1, 5, 3),
        req("TIM2", Tx, Dma1, 6, 3),
        // TIM3 (channel 5): CH4/UP s2, CH1/TRIG s4, CH2 s5, CH3 s7.
        req("TIM3", Rx, Dma1, 4, 5),
        req("TIM3", Tx, Dma1, 5, 5),
        // TIM5 (channel 6): CH3/UP s0, CH4/TRIG s1, CH1 s2, CH2 s4, CH4 s6.
        req("TIM5", Rx, Dma1, 0, 6),
        req("TIM5", Tx, Dma1, 1, 6),
        // SPI2 (channel 0): RX stream 3, TX stream 4.
        req("SPI2", Rx, Dma1, 3, 0),
        req("SPI2", Tx, Dma1, 4, 0),
        // I2C2 (channel 7): RX stream 2/3, TX stream 7.
        req("I2C2", Rx, Dma1, 2, 7),
        req("I2C2", Rx, Dma1, 3, 7),
        req("I2C2", Tx, Dma1, 7, 7),
        // ---- DMA2 (RM0390 Table 29) ----
        // ADC1 (channel 0): stream 0 and stream 4.
        req("ADC1", Rx, Dma2, 0, 0),
        req("ADC1", Rx, Dma2, 4, 0),
        // SPI1 (channel 3): RX stream 0/2, TX stream 3/5.
        req("SPI1", Rx, Dma2, 0, 3),
        req("SPI1", Rx, Dma2, 2, 3),
        req("SPI1", Tx, Dma2, 3, 3),
        req("SPI1", Tx, Dma2, 5, 3),
        // USART1 (channel 4): RX stream 2/5, TX stream 7.
        req("USART1", Rx, Dma2, 2, 4),
        req("USART1", Rx, Dma2, 5, 4),
        req("USART1", Tx, Dma2, 7, 4),
        // USART6 (channel 5): RX stream 1/2, TX stream 6/7.
        req("USART6", Rx, Dma2, 1, 5),
        req("USART6", Rx, Dma2, 2, 5),
        req("USART6", Tx, Dma2, 6, 5),
        req("USART6", Tx, Dma2, 7, 5),
    ];

    // --- STM32F411RE (RM0383 Table 27 = DMA1, Table 28 = DMA2) ---------------
    //
    // The F411 is a smaller package: no UART4/5, no USART3, no TIM-on-DMA rows
    // for the higher timers are modeled here, and only ADC1 exists. The shared
    // peripherals (SPI1/2/3, I2C1/2/3, USART1/2/6, TIM2-5, ADC1) keep the same
    // stream/channel assignments as the F446 — the DMA request tables are
    // identical for the parts both families share.
    pub(super) const F411RE: &[DmaRequest] = &[
        // ---- DMA1 (RM0383 Table 27) ----
        // SPI3 (channel 0).
        req("SPI3", Rx, Dma1, 0, 0),
        req("SPI3", Rx, Dma1, 2, 0),
        req("SPI3", Tx, Dma1, 5, 0),
        req("SPI3", Tx, Dma1, 7, 0),
        // I2C1 (channel 1).
        req("I2C1", Rx, Dma1, 0, 1),
        req("I2C1", Rx, Dma1, 5, 1),
        req("I2C1", Tx, Dma1, 6, 1),
        req("I2C1", Tx, Dma1, 7, 1),
        // TIM4 (channel 2).
        req("TIM4", Rx, Dma1, 0, 2),
        req("TIM4", Tx, Dma1, 6, 2),
        // I2C3 (channel 3).
        req("I2C3", Rx, Dma1, 1, 3),
        req("I2C3", Rx, Dma1, 2, 3),
        req("I2C3", Tx, Dma1, 4, 3),
        // USART2 (channel 4).
        req("USART2", Rx, Dma1, 5, 4),
        req("USART2", Tx, Dma1, 6, 4),
        // TIM2 (channel 3).
        req("TIM2", Rx, Dma1, 5, 3),
        req("TIM2", Tx, Dma1, 6, 3),
        // TIM3 (channel 5).
        req("TIM3", Rx, Dma1, 4, 5),
        req("TIM3", Tx, Dma1, 5, 5),
        // TIM5 (channel 6).
        req("TIM5", Rx, Dma1, 0, 6),
        req("TIM5", Tx, Dma1, 1, 6),
        // SPI2 (channel 0).
        req("SPI2", Rx, Dma1, 3, 0),
        req("SPI2", Tx, Dma1, 4, 0),
        // I2C2 (channel 7).
        req("I2C2", Rx, Dma1, 2, 7),
        req("I2C2", Rx, Dma1, 3, 7),
        req("I2C2", Tx, Dma1, 7, 7),
        // ---- DMA2 (RM0383 Table 28) ----
        // ADC1 (channel 0).
        req("ADC1", Rx, Dma2, 0, 0),
        req("ADC1", Rx, Dma2, 4, 0),
        // SPI1 (channel 3).
        req("SPI1", Rx, Dma2, 0, 3),
        req("SPI1", Rx, Dma2, 2, 3),
        req("SPI1", Tx, Dma2, 3, 3),
        req("SPI1", Tx, Dma2, 5, 3),
        // USART1 (channel 4).
        req("USART1", Rx, Dma2, 2, 4),
        req("USART1", Rx, Dma2, 5, 4),
        req("USART1", Tx, Dma2, 7, 4),
        // USART6 (channel 5).
        req("USART6", Rx, Dma2, 1, 5),
        req("USART6", Rx, Dma2, 2, 5),
        req("USART6", Tx, Dma2, 6, 5),
        req("USART6", Tx, Dma2, 7, 5),
    ];
}
