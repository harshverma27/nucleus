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
fn sysclk_sources_present() {
    for ct in [ClockTree::f446re(), ClockTree::f411re()] {
        let srcs = ct.sysclk_sources();
        assert!(srcs.contains(&SysclkSource::Pll));
        assert!(srcs.contains(&SysclkSource::Hse));
        assert!(srcs.contains(&SysclkSource::Hsi));
    }
}
