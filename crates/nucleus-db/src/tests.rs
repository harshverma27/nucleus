use super::*;

// --- Pin parsing ---------------------------------------------------------

#[test]
fn parses_pin_name() {
    assert_eq!("PA7".parse::<Pin>(), Ok(Pin::new(Port::A, 7)));
    assert_eq!("PC13".parse::<Pin>(), Ok(Pin::new(Port::C, 13)));
}

#[test]
fn rejects_malformed_pin_names() {
    assert!("".parse::<Pin>().is_err());
    assert!("A7".parse::<Pin>().is_err()); // missing leading P
    assert!("PZ1".parse::<Pin>().is_err()); // no port Z
    assert!("PA".parse::<Pin>().is_err()); // missing number
    assert!("PA7x".parse::<Pin>().is_err()); // trailing junk
}

#[test]
fn pin_round_trips_through_display() {
    let pin = "PB9".parse::<Pin>().unwrap();
    assert_eq!(pin.to_string(), "PB9");
}

#[test]
#[should_panic(expected = "out of range")]
fn pin_new_rejects_out_of_range_number_like_from_str_does() {
    Pin::new(Port::A, 16);
}

#[test]
fn pin_new_accepts_the_full_valid_range() {
    for number in 0..=15 {
        Pin::new(Port::A, number);
    }
}

// --- Forward lookup (the Phase 1 exit-criteria test) ---------------------

#[test]
fn pa7_af5_is_spi1_mosi() {
    let db = Database::f446re();
    let pin = "PA7".parse::<Pin>().unwrap();

    // One (pin, AF) can carry several signals (SPI/I2S share AF numbers);
    // SPI1_MOSI must be among PA7's AF5 mappings.
    let signals: Vec<_> = db
        .lookup(pin, 5)
        .map(|m| (m.peripheral, m.signal))
        .collect();

    assert!(
        signals.contains(&("SPI1", "MOSI")),
        "PA7 AF5 should include SPI1_MOSI, got {signals:?}"
    );
}

#[test]
fn unmapped_af_yields_nothing() {
    let db = Database::f446re();
    let pin = "PA7".parse::<Pin>().unwrap();

    // PA7 has no AF0 (system) function on the F446RE.
    assert_eq!(db.lookup(pin, 0).count(), 0);
}

#[test]
fn lists_all_alt_functions_for_a_pin() {
    let db = Database::f446re();
    let pin = "PA2".parse::<Pin>().unwrap();

    let signals: Vec<_> = db
        .alt_functions(pin)
        .map(|m| (m.peripheral, m.signal))
        .collect();

    assert!(signals.contains(&("USART2", "TX")));
}

// --- Reverse lookup (used by the constraint solver) ----------------------

#[test]
fn reverse_lookup_finds_af_number() {
    let db = Database::f446re();
    let pin = "PA5".parse::<Pin>().unwrap();

    assert_eq!(db.find_af(pin, "SPI1", "SCK"), Some(5));
}

#[test]
fn reverse_lookup_missing_signal_is_none() {
    let db = Database::f446re();
    let pin = "PA5".parse::<Pin>().unwrap();

    assert_eq!(db.find_af(pin, "I2C1", "SDA"), None);
}

// --- Candidate pins lookup (used by the auto-router M4) -------------------

#[test]
fn candidate_pins_finds_known_peripheral_signal() {
    let db = Database::f446re();

    // SPI1 MOSI is available on PA7 (and other pins).
    let pins = db.candidate_pins("SPI1", "MOSI");

    assert!(
        pins.iter().any(|p| p.port == Port::A && p.number == 7),
        "SPI1_MOSI should include PA7, got {:?}",
        pins
    );
}

#[test]
fn candidate_pins_returns_empty_for_unmapped_signal() {
    let db = Database::f446re();

    // An unmodeled peripheral/signal combo returns empty, never panics.
    let pins = db.candidate_pins("MADEUP", "SIGNAL");

    assert_eq!(
        pins,
        vec![],
        "unmodeled peripheral+signal should return empty vec"
    );
}

#[test]
fn candidate_pins_returns_empty_for_unmapped_peripheral() {
    let db = Database::f446re();

    // SPI1 has no INVALID signal.
    let pins = db.candidate_pins("SPI1", "INVALID");

    assert_eq!(
        pins,
        vec![],
        "unmapped signal on known peripheral should return empty vec"
    );
}

#[test]
fn candidate_pins_is_sorted_and_deduplicated() {
    let db = Database::f446re();

    // Get candidate pins for a signal that should appear on multiple pins.
    let pins = db.candidate_pins("USART2", "TX");

    // Result should be sorted.
    for i in 1..pins.len() {
        assert!(
            pins[i - 1] <= pins[i],
            "candidate pins should be sorted, got {pins:?}"
        );
    }

    // No duplicates.
    for i in 1..pins.len() {
        assert_ne!(
            pins[i - 1],
            pins[i],
            "candidate pins should be deduplicated, got {pins:?}"
        );
    }
}

#[test]
fn candidate_pins_agrees_with_find_af_forward_lookup() {
    let db = Database::f446re();

    // Cross-validation: if candidate_pins returns a pin, that pin must be
    // reachable via find_af with the same peripheral+signal.
    let candidates = db.candidate_pins("SPI1", "MOSI");
    for pin in candidates {
        let af = db.find_af(pin, "SPI1", "MOSI");
        assert!(
            af.is_some(),
            "candidate pin {} for SPI1_MOSI should be reachable via find_af, got {:?}",
            pin,
            af
        );
    }
}

#[test]
fn candidate_pins_f411_differs_from_f446() {
    let f446 = Database::f446re();
    let f411 = Database::f411re();

    // UART5 is on F446 but not F411; UART5 TX should have candidates on F446
    // but not on F411.
    let f446_candidates = f446.candidate_pins("UART5", "TX");
    let f411_candidates = f411.candidate_pins("UART5", "TX");

    assert!(
        !f446_candidates.is_empty(),
        "F446 should have UART5_TX candidates"
    );
    assert_eq!(
        f411_candidates,
        vec![],
        "F411 should have no UART5_TX candidates (UART5 not present)"
    );
}

// --- Pack parser (CMSIS/CubeMX open pin data XML) -------------------------

const GPIO_MODES_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<IP Name="GPIO" Version="STM32F446_gpio_v1_0" xmlns="http://dummy.com">
    <GPIO_Pin PortName="PA" Name="PA7">
        <SpecificParameter Name="GPIO_Pin">
            <PossibleValue>GPIO_PIN_7</PossibleValue>
        </SpecificParameter>
        <PinSignal Name="SPI1_MOSI">
            <SpecificParameter Name="GPIO_AF">
                <PossibleValue>GPIO_AF5_SPI1</PossibleValue>
            </SpecificParameter>
        </PinSignal>
        <PinSignal Name="TIM3_CH2">
            <SpecificParameter Name="GPIO_AF">
                <PossibleValue>GPIO_AF2_TIM3</PossibleValue>
            </SpecificParameter>
        </PinSignal>
    </GPIO_Pin>
    <GPIO_Pin PortName="PA" Name="PA13">
        <PinSignal Name="SYS_JTMS-SWDIO">
            <SpecificParameter Name="GPIO_AF">
                <PossibleValue>GPIO_AF0_SYS</PossibleValue>
            </SpecificParameter>
        </PinSignal>
    </GPIO_Pin>
    <GPIO_Pin PortName="PA" Name="PA15">
        <PinSignal Name="CEC">
            <SpecificParameter Name="GPIO_AF">
                <PossibleValue>GPIO_AF4_CEC</PossibleValue>
            </SpecificParameter>
        </PinSignal>
    </GPIO_Pin>
    <GPIO_Pin PortName="PDR_ON" Name="PDR_ON">
        <PinSignal Name="SYS_PDR_ON">
            <SpecificParameter Name="GPIO_AF">
                <PossibleValue>GPIO_AF0_SYS</PossibleValue>
            </SpecificParameter>
        </PinSignal>
    </GPIO_Pin>
</IP>"#;

const MCU_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<Mcu Family="STM32F4" Line="STM32F446" RefName="STM32F446R(C-E)Tx" xmlns="http://dummy.com">
    <Pin Name="VBAT" Position="1" Type="Power"/>
    <Pin Name="PC13" Position="2" Type="I/O"/>
    <Pin Name="PC14-OSC32_IN" Position="3" Type="I/O"/>
    <Pin Name="NRST" Position="7" Type="Reset"/>
    <Pin Name="PA7" Position="23" Type="I/O"/>
</Mcu>"#;

#[test]
fn parses_gpio_modes_xml() {
    let mappings = pack::parse_gpio_modes(GPIO_MODES_FIXTURE).unwrap();

    assert_eq!(mappings.len(), 4);
    let m = &mappings[0];
    assert_eq!(
        (
            m.pin.as_str(),
            m.af,
            m.peripheral.as_str(),
            m.signal.as_str()
        ),
        ("PA7", 5, "SPI1", "MOSI")
    );
    // Signals containing dashes keep everything after the first underscore.
    let swdio = mappings.iter().find(|m| m.pin == "PA13").unwrap();
    assert_eq!(
        (swdio.af, swdio.peripheral.as_str(), swdio.signal.as_str()),
        (0, "SYS", "JTMS-SWDIO")
    );
    // Known anomaly: peripheral-only signal names (the F446's HDMI-CEC) have
    // no PERIPH_SIGNAL form and normalize to peripheral == signal.
    let cec = mappings.iter().find(|m| m.pin == "PA15").unwrap();
    assert_eq!(
        (cec.af, cec.peripheral.as_str(), cec.signal.as_str()),
        (4, "CEC", "CEC")
    );
    // Known anomaly: non-GPIO entries (the F446's PDR_ON ball) are skipped.
    assert!(!mappings.iter().any(|m| m.pin.contains("PDR")));
}

#[test]
fn parses_package_pins_and_normalizes_names() {
    let pins = pack::parse_package_pins(MCU_FIXTURE).unwrap();

    // Only GPIO pins, with oscillator suffixes stripped; power/reset excluded.
    assert_eq!(pins, vec!["PC13", "PC14", "PA7"]);
}

#[test]
fn patches_add_and_remove_mappings() {
    let mut mappings = pack::parse_gpio_modes(GPIO_MODES_FIXTURE).unwrap();
    let patches = [
        pack::Patch::Remove {
            pin: "PA7",
            af: 2,
            peripheral: "TIM3",
            signal: "CH2",
            reason: "test",
        },
        pack::Patch::Add {
            pin: "PA7",
            af: 9,
            peripheral: "TIM14",
            signal: "CH1",
            reason: "test",
        },
    ];

    pack::apply_patches(&mut mappings, &patches);

    assert!(!mappings.iter().any(|m| m.peripheral == "TIM3"));
    assert!(mappings
        .iter()
        .any(|m| m.peripheral == "TIM14" && m.af == 9));
}

#[test]
fn generated_table_is_deterministic_filtered_and_sorted() {
    let mappings = pack::parse_gpio_modes(GPIO_MODES_FIXTURE).unwrap();
    let pins = vec!["PA7".to_string()]; // PA13 not in package -> filtered out

    let a = pack::generate_table(&mappings, &pins, "TEST");
    let b = pack::generate_table(&mappings, &pins, "TEST");

    assert_eq!(a, b, "generation must be byte-deterministic");
    assert!(
        !a.contains("PA13") && !a.contains("JTMS"),
        "non-package pins filtered"
    );
    // Sorted by AF: TIM3 (AF2) before SPI1 (AF5).
    let tim3 = a.find("TIM3").unwrap();
    let spi1 = a.find("SPI1").unwrap();
    assert!(tim3 < spi1, "entries sorted by (pin, af)");
}

// --- Generated full-device database ---------------------------------------

#[test]
fn generated_db_agrees_with_hand_verified_seed() {
    // Every datasheet-verified seed entry must appear identically in the
    // database generated from ST's pin data (cross-validation of sources).
    let db = Database::f446re();
    for seed in data::SEED {
        assert!(
            db.lookup(seed.pin, seed.af)
                .any(|m| (m.peripheral, m.signal) == (seed.peripheral, seed.signal)),
            "seed entry {} AF{} = {}_{} missing from generated DB",
            seed.pin,
            seed.af,
            seed.peripheral,
            seed.signal
        );
    }
}

#[test]
fn generated_db_covers_full_package() {
    let db = Database::f446re();

    // LQFP64 exposes ~50 GPIOs; the seed had 10 entries on 9 pins.
    let mut pins: Vec<Pin> = db.entries.iter().map(|m| m.pin).collect();
    pins.sort();
    pins.dedup();
    assert!(
        pins.len() >= 45,
        "expected full-package coverage, got {} pins",
        pins.len()
    );
    assert!(
        db.entries.len() >= 100,
        "expected >=100 mappings, got {}",
        db.entries.len()
    );

    // Debug pins present (not in the seed).
    let pa13 = "PA13".parse::<Pin>().unwrap();
    assert!(
        db.lookup(pa13, 0)
            .any(|m| (m.peripheral, m.signal) == ("SYS", "JTMS-SWDIO")),
        "PA13 AF0 should include SYS_JTMS-SWDIO"
    );
}

// --- STM32F411RE ----------------------------------------------------------

#[test]
fn f411re_seed_cross_validation() {
    // Every datasheet-verified F411 seed entry must appear identically in the
    // table generated from ST's pin data (cross-validation of sources).
    let db = Database::f411re();
    for seed in data::SEED_F411RE {
        assert!(
            db.lookup(seed.pin, seed.af)
                .any(|m| (m.peripheral, m.signal) == (seed.peripheral, seed.signal)),
            "F411 seed entry {} AF{} = {}_{} missing from generated DB",
            seed.pin,
            seed.af,
            seed.peripheral,
            seed.signal
        );
    }
}

#[test]
fn f411re_covers_full_package() {
    let db = Database::f411re();

    let mut pins: Vec<Pin> = db.entries.iter().map(|m| m.pin).collect();
    pins.sort();
    pins.dedup();
    assert!(
        pins.len() >= 35,
        "expected full-package coverage, got {} pins",
        pins.len()
    );
    assert!(
        db.entries.len() >= 150,
        "expected >=150 mappings, got {}",
        db.entries.len()
    );

    // Debug pin present.
    let pa13 = "PA13".parse::<Pin>().unwrap();
    assert!(
        db.lookup(pa13, 0)
            .any(|m| (m.peripheral, m.signal) == ("SYS", "JTMS-SWDIO")),
        "PA13 AF0 should include SYS_JTMS-SWDIO"
    );
}

#[test]
fn has_peripheral_differs_by_family() {
    let f411 = Database::f411re();
    let f446 = Database::f446re();

    // Shared peripheral present on both.
    assert!(f411.has_peripheral("USART2"));
    assert!(f446.has_peripheral("USART2"));

    // UART4 exists on F446 but not on the F411RE package.
    assert!(f446.has_peripheral("UART4"));
    assert!(!f411.has_peripheral("UART4"));
}

// --- Clock-tree model (M1) -----------------------------------------------
//
// The clock tree has no pack source to cross-validate against, so the oracle is
// the reference manual itself: every value below is typed by hand from RM0390
// (F446) / RM0383 (F411) and the model must agree with it.

use clock::{Bus, ClockTree, Oscillator, SysclkSource};

#[test]
fn clock_tree_f446_matches_rm0390_seed() {
    let ct = ClockTree::f446re();

    // Oscillators (RM0390 §6.2): HSI 16 MHz, LSI 32 kHz, LSE 32.768 kHz,
    // HSE nominal 8 MHz with the 4–26 MHz crystal range.
    assert_eq!(
        ct.oscillator(Oscillator::Hsi).unwrap().nominal_hz,
        16_000_000
    );
    assert_eq!(ct.oscillator(Oscillator::Lsi).unwrap().nominal_hz, 32_000);
    assert_eq!(ct.oscillator(Oscillator::Lse).unwrap().nominal_hz, 32_768);
    let hse = ct.oscillator(Oscillator::Hse).unwrap();
    assert_eq!(hse.nominal_hz, 8_000_000);
    assert_eq!(
        (hse.range.min_hz, hse.range.max_hz),
        (4_000_000, 26_000_000)
    );

    // Main PLL (RM0390 §6.3.2): M 2..=63, N 50..=432, P {2,4,6,8}, Q 2..=15,
    // VCO_in 1–2 MHz, VCO_out 100–432 MHz.
    let pll = ct.pll();
    assert_eq!((pll.m.min, pll.m.max), (2, 63));
    assert_eq!((pll.n.min, pll.n.max), (50, 432));
    assert_eq!(pll.p, &[2, 4, 6, 8]);
    assert_eq!((pll.q.min, pll.q.max), (2, 15));
    assert_eq!(
        (pll.vco_in.min_hz, pll.vco_in.max_hz),
        (1_000_000, 2_000_000)
    );
    assert_eq!(
        (pll.vco_out.min_hz, pll.vco_out.max_hz),
        (100_000_000, 432_000_000)
    );

    // Bus limits (RM0390 §6.2).
    let lim = ct.limits();
    assert_eq!(lim.max_sysclk_hz, 180_000_000);
    assert_eq!(lim.max_ahb_hz, 180_000_000);
    assert_eq!(lim.max_apb1_hz, 45_000_000);
    assert_eq!(lim.max_apb2_hz, 90_000_000);

    // Prescaler sets (RM0390 §6.3.3).
    assert_eq!(ct.ahb_prescalers(), &[1, 2, 4, 8, 16, 64, 128, 256, 512]);
    assert_eq!(ct.prescalers(Bus::Apb1), &[1, 2, 4, 8, 16]);
    assert_eq!(ct.prescalers(Bus::Apb2), &[1, 2, 4, 8, 16]);

    // Peripheral bus derivation mirrors the compiler's model.
    assert_eq!(ct.peripheral_bus("USART2"), Some(Bus::Apb1));
    assert_eq!(ct.peripheral_bus("SPI1"), Some(Bus::Apb2));
    assert_eq!(ct.peripheral_bus("UART4"), Some(Bus::Apb1));
    assert_eq!(ct.peripheral_bus("MADEUP"), None);
}

#[test]
fn clock_tree_f411_matches_rm0383_seed() {
    let ct = ClockTree::f411re();

    // Oscillators identical to the F446 NUCLEO board.
    assert_eq!(
        ct.oscillator(Oscillator::Hsi).unwrap().nominal_hz,
        16_000_000
    );
    assert_eq!(
        ct.oscillator(Oscillator::Hse).unwrap().nominal_hz,
        8_000_000
    );

    // Main PLL (RM0383 §6.3.2): same divider ranges as the F446.
    let pll = ct.pll();
    assert_eq!((pll.m.min, pll.m.max), (2, 63));
    assert_eq!((pll.n.min, pll.n.max), (50, 432));
    assert_eq!(
        (pll.vco_out.min_hz, pll.vco_out.max_hz),
        (100_000_000, 432_000_000)
    );

    // Bus limits (RM0383 §6.2) — lower than the F446.
    let lim = ct.limits();
    assert_eq!(lim.max_sysclk_hz, 100_000_000);
    assert_eq!(lim.max_ahb_hz, 100_000_000);
    assert_eq!(lim.max_apb1_hz, 50_000_000);
    assert_eq!(lim.max_apb2_hz, 100_000_000);

    // The F411 package omits UART4/5 and USART3.
    assert_eq!(ct.peripheral_bus("USART2"), Some(Bus::Apb1));
    assert_eq!(ct.peripheral_bus("SPI1"), Some(Bus::Apb2));
    assert_eq!(ct.peripheral_bus("UART4"), None);
    assert_eq!(ct.peripheral_bus("USART3"), None);
}

#[test]
fn silicon_limits_differ_by_family() {
    // The headline family difference: the F446 runs to 180 MHz, the F411 to 100.
    let f446 = ClockTree::f446re().limits();
    let f411 = ClockTree::f411re().limits();
    assert!(f446.max_sysclk_hz > f411.max_sysclk_hz);
    assert_eq!(f446.max_apb1_hz, 45_000_000);
    assert_eq!(f411.max_apb1_hz, 50_000_000);
    assert_ne!(f446.max_apb2_hz, f411.max_apb2_hz);
}

#[test]
fn apbx_timer_x2_rule() {
    // Timers run at the APB clock when the prescaler is 1, doubled otherwise
    // (RM0390/RM0383 §6.2). The AHB bus has no doubling rule.
    assert_eq!(ClockTree::timer_multiplier(Bus::Apb1, 1), 1);
    assert_eq!(ClockTree::timer_multiplier(Bus::Apb1, 2), 2);
    assert_eq!(ClockTree::timer_multiplier(Bus::Apb2, 4), 2);
    assert_eq!(ClockTree::timer_multiplier(Bus::Apb2, 1), 1);
    assert_eq!(ClockTree::timer_multiplier(Bus::Ahb1, 8), 1);
}

#[test]
fn without_oscillator_drops_only_the_named_source() {
    let base = ClockTree::f446re();
    let no_hse = ClockTree::without_oscillator(base, Oscillator::Hse);
    assert!(no_hse.oscillator(Oscillator::Hse).is_none());
    // Everything else from `base` carries over unchanged.
    assert!(no_hse.oscillator(Oscillator::Hsi).is_some());
    assert_eq!(no_hse.limits().max_sysclk_hz, base.limits().max_sysclk_hz);
    assert_eq!(no_hse.pll().m.min, base.pll().m.min);
}

#[test]
fn sysclk_sources_present() {
    for ct in [ClockTree::f446re(), ClockTree::f411re()] {
        let srcs = ct.sysclk_sources();
        assert!(srcs.contains(&SysclkSource::Pll));
        assert!(srcs.contains(&SysclkSource::Hse));
        assert!(srcs.contains(&SysclkSource::Hsi));
    }
}

// --- DMA request-map model (M2) ------------------------------------------
//
// Like the clock tree, the DMA request map has no pack source to cross-validate
// against, so the oracle is the reference manual itself: every slot below is
// hand-typed from RM0390 (F446, DMA1 Table 28 / DMA2 Table 29) and RM0383 (F411,
// DMA1 Table 27 / DMA2 Table 28), and the model must agree with it.

use dma::{Controller, Direction, DmaMap, Slot};

#[test]
fn dma_map_f446_matches_rm0390_seed() {
    let m = DmaMap::f446re();

    // RM0390 Table 28 (DMA1): USART2_RX is DMA1 stream 5 channel 4,
    // USART2_TX is DMA1 stream 6 channel 4.
    assert_eq!(
        m.candidates("USART2", Direction::Rx),
        vec![Slot::new(Controller::Dma1, 5, 4)]
    );
    assert_eq!(
        m.candidates("USART2", Direction::Tx),
        vec![Slot::new(Controller::Dma1, 6, 4)]
    );

    // RM0390 Table 29 (DMA2): SPI1_RX is on channel 3, streams 0 and 2 (the
    // two-slot alternative the solver relies on); SPI1_TX on streams 3 and 5.
    assert_eq!(
        m.candidates("SPI1", Direction::Rx),
        vec![
            Slot::new(Controller::Dma2, 0, 3),
            Slot::new(Controller::Dma2, 2, 3),
        ]
    );
    assert_eq!(
        m.candidates("SPI1", Direction::Tx),
        vec![
            Slot::new(Controller::Dma2, 3, 3),
            Slot::new(Controller::Dma2, 5, 3),
        ]
    );

    // RM0390 Table 29: ADC1 is DMA2 channel 0, streams 0 and 4.
    assert_eq!(
        m.candidates("ADC1", Direction::Rx),
        vec![
            Slot::new(Controller::Dma2, 0, 0),
            Slot::new(Controller::Dma2, 4, 0),
        ]
    );

    // RM0390 Table 28: SPI3_RX channel 0, streams 0 and 2.
    assert_eq!(
        m.candidates("SPI3", Direction::Rx),
        vec![
            Slot::new(Controller::Dma1, 0, 0),
            Slot::new(Controller::Dma1, 2, 0),
        ]
    );

    // USART1 is an F446 (and F411) DMA2 peripheral; UART5 is F446-only.
    assert!(m.has_peripheral("USART1"));
    assert!(m.has_peripheral("UART5"));
    // Unmodeled peripheral yields no candidates (never a false positive).
    assert!(m.candidates("MADEUP", Direction::Tx).is_empty());
}

#[test]
fn dma_map_f411_matches_rm0383_seed() {
    let m = DmaMap::f411re();

    // RM0383 Table 27 (DMA1): shared peripherals keep the F446 assignments.
    assert_eq!(
        m.candidates("USART2", Direction::Tx),
        vec![Slot::new(Controller::Dma1, 6, 4)]
    );
    assert_eq!(
        m.candidates("I2C1", Direction::Rx),
        vec![
            Slot::new(Controller::Dma1, 0, 1),
            Slot::new(Controller::Dma1, 5, 1),
        ]
    );

    // RM0383 Table 28 (DMA2): USART1_RX channel 4, streams 2 and 5.
    assert_eq!(
        m.candidates("USART1", Direction::Rx),
        vec![
            Slot::new(Controller::Dma2, 2, 4),
            Slot::new(Controller::Dma2, 5, 4),
        ]
    );

    // The F411 package omits UART4/5 and USART3 — no DMA rows for them.
    assert!(!m.has_peripheral("UART4"));
    assert!(!m.has_peripheral("UART5"));
    assert!(!m.has_peripheral("USART3"));
    assert!(m.candidates("UART5", Direction::Tx).is_empty());
}

#[test]
fn dma_request_map_differs_by_family() {
    let f446 = DmaMap::f446re();
    let f411 = DmaMap::f411re();

    // Shared peripheral present on both with identical slots.
    assert!(f446.has_peripheral("SPI1") && f411.has_peripheral("SPI1"));
    assert_eq!(
        f446.candidates("SPI1", Direction::Tx),
        f411.candidates("SPI1", Direction::Tx)
    );

    // UART5 exists only on the F446.
    assert!(f446.has_peripheral("UART5"));
    assert!(!f411.has_peripheral("UART5"));
}

#[test]
fn dma_slots_are_in_range_and_deterministic() {
    // Every modeled slot uses a valid stream (0..=7) and channel (0..=7), and
    // candidate enumeration preserves table order (determinism for the solver).
    for m in [DmaMap::f446re(), DmaMap::f411re()] {
        for r in m.requests() {
            assert!(r.slot.stream <= 7, "stream out of range: {:?}", r);
            assert!(r.slot.channel <= 7, "channel out of range: {:?}", r);
        }
        // Two enumerations of the same request are byte-identical.
        let a = m.candidates("SPI1", Direction::Rx);
        let b = m.candidates("SPI1", Direction::Rx);
        assert_eq!(a, b);
    }
}

#[test]
fn dma_direction_resolves_spi_signal_aliases() {
    // SPI MOSI/MISO are the TX/RX DMA paths; the solver resolves an stm32.toml
    // `mosi`/`miso` line to the right direction.
    assert_eq!(Direction::from_signal("MOSI"), Some(Direction::Tx));
    assert_eq!(Direction::from_signal("MISO"), Some(Direction::Rx));
    assert_eq!(Direction::from_signal("TX"), Some(Direction::Tx));
    assert_eq!(Direction::from_signal("RX"), Some(Direction::Rx));
    assert_eq!(Direction::from_signal("SCK"), None);
}

// --- IRQ/NVIC vector map model (M3) ---------------------------------------
//
// Like the clock tree and DMA request map, the IRQ vector map has no pack
// source to cross-validate against, so the oracle is the reference manual
// itself: every vector below is hand-typed from RM0390 (F446, §10.1.2 Table
// 38) and RM0383 (F411, §10.1.2 vector table), and the model must agree
// with it.

use irq::{ExtiGroups, IrqMap};

#[test]
fn irq_map_f446_matches_rm0390_seed() {
    let m = IrqMap::f446re();

    // RM0390 Table 38: USART1/2/3, UART4/5, USART6 each have one vector
    // sharing their own name.
    assert_eq!(m.vectors("USART1"), &["USART1"]);
    assert_eq!(m.vectors("UART4"), &["UART4"]);
    assert_eq!(m.vectors("UART5"), &["UART5"]);
    assert_eq!(m.vectors("USART6"), &["USART6"]);

    // SPI1-4 each have one vector.
    assert_eq!(m.vectors("SPI1"), &["SPI1"]);
    assert_eq!(m.vectors("SPI4"), &["SPI4"]);

    // I2Cx has two vectors: event and error.
    assert_eq!(m.vectors("I2C1"), &["I2C1_EV", "I2C1_ER"]);
    assert_eq!(m.vectors("I2C3"), &["I2C3_EV", "I2C3_ER"]);

    // TIM2-5 each have one vector.
    assert_eq!(m.vectors("TIM2"), &["TIM2"]);
    assert_eq!(m.vectors("TIM5"), &["TIM5"]);

    assert!(m.has_peripheral("USART3"));
    assert!(m.has_peripheral("UART5"));
    // Unmodeled peripheral yields no vectors (never a false positive).
    assert!(m.vectors("MADEUP").is_empty());
}

#[test]
fn irq_map_f411_matches_rm0383_seed() {
    let m = IrqMap::f411re();

    // Shared peripherals keep the F446 vector names.
    assert_eq!(m.vectors("USART1"), &["USART1"]);
    assert_eq!(m.vectors("USART2"), &["USART2"]);
    assert_eq!(m.vectors("USART6"), &["USART6"]);
    assert_eq!(m.vectors("I2C2"), &["I2C2_EV", "I2C2_ER"]);
    assert_eq!(m.vectors("TIM3"), &["TIM3"]);

    // The F411 package omits USART3 and UART4/5 — no IRQ rows for them
    // (mirrors the same omission in the DMA and clock-tree models).
    assert!(!m.has_peripheral("USART3"));
    assert!(!m.has_peripheral("UART4"));
    assert!(!m.has_peripheral("UART5"));
    assert!(m.vectors("UART5").is_empty());
}

#[test]
fn irq_vector_map_differs_by_family() {
    let f446 = IrqMap::f446re();
    let f411 = IrqMap::f411re();

    // Shared peripheral present on both with identical vectors.
    assert!(f446.has_peripheral("SPI1") && f411.has_peripheral("SPI1"));
    assert_eq!(f446.vectors("SPI1"), f411.vectors("SPI1"));

    // UART5 exists only on the F446.
    assert!(f446.has_peripheral("UART5"));
    assert!(!f411.has_peripheral("UART5"));
}

#[test]
fn exti_group_boundaries_are_correct() {
    // Lines 0-4 are individually vectored.
    assert_eq!(irq::group_for(0), "EXTI0");
    assert_eq!(irq::group_for(4), "EXTI4");
    // Lines 5-9 share EXTI9_5.
    assert_eq!(irq::group_for(5), "EXTI9_5");
    assert_eq!(irq::group_for(9), "EXTI9_5");
    // Lines 10-15 share EXTI15_10.
    assert_eq!(irq::group_for(10), "EXTI15_10");
    assert_eq!(irq::group_for(15), "EXTI15_10");
}

#[test]
fn exti_groups_identical_across_families() {
    // The EXTI/NVIC layout is identical on both families (same RM0390/RM0383
    // vector table), so ExtiGroups should agree for every line.
    let f446 = ExtiGroups::f446re();
    let f411 = ExtiGroups::f411re();
    for line in 0..=15u8 {
        assert_eq!(f446.group_for(line), f411.group_for(line));
    }
}
