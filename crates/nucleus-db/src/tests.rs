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

    let mapping = db.lookup(pin, 5).expect("PA7 AF5 should be mapped");

    assert_eq!(mapping.peripheral, "SPI1");
    assert_eq!(mapping.signal, "MOSI");
}

#[test]
fn unmapped_af_returns_none() {
    let db = Database::f446re();
    let pin = "PA7".parse::<Pin>().unwrap();

    // AF0 is not an SPI1 function on PA7 in the seed data.
    assert!(db.lookup(pin, 0).is_none());
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
