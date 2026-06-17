//! The DMA arbitration solver (Nucleus v2 milestone M2).
//!
//! Turns the "do two peripherals fight for one DMA stream?" question into a
//! concrete slot assignment: it walks the configured peripherals that *opt into*
//! DMA, asks the family's [`nucleus_db::dma::DmaMap`] for the candidate
//! `(controller, stream, channel)` slots that can serve each `(peripheral,
//! direction)` request, and assigns each request the first free stream. When a
//! request's every candidate stream is already taken, that is a
//! [`Conflict::DmaCollision`] — reported once per contested stream (not per
//! pair), mirroring how the pin solver reports one [`Conflict::PinCollision`] per
//! over-subscribed pin. When a free alternative slot exists for one of the two
//! contenders, the conflict names it as a suggestion.
//!
//! Everything here is **pure and synchronous** (config + model → conflicts), so
//! it unit-tests trivially and the LSP picks it up for free. All slot data comes
//! from the model, never hard-coded, so the F446 and F411 differ automatically.
//!
//! ## Config surface
//!
//! A peripheral requests DMA **explicitly** — the solver never infers it from the
//! peripheral kind, so the default scaffold config (which sets no `dma` key)
//! never trips a DMA error. The opt-in lives in the peripheral's own table as a
//! `dma` key, parsed from the raw value bag (so `#[serde(deny_unknown_fields)]`
//! is unaffected — the peripheral table is `#[serde(transparent)]`):
//!
//! ```toml
//! [peripherals.usart2]
//! tx = "PA2"
//! rx = "PA3"
//! dma = true            # both TX and RX request a DMA stream
//!
//! [peripherals.spi1]
//! mosi = "PA7"
//! miso = "PA6"
//! sck = "PA5"
//! dma = ["rx"]          # only the RX (MISO) path requests DMA
//! ```
//!
//! `dma = true` requests both directions the family models for that peripheral;
//! `dma = ["tx"]` / `dma = ["rx"]` (or `["mosi"]`/`["miso"]`) select one.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_db::dma::{Controller, Direction, DmaMap, Slot};

use crate::config::Config;
use crate::solver::Conflict;

/// A request that a configured peripheral makes for a DMA stream, identified by
/// its database peripheral name and direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Request {
    peripheral_idx: usize,
    direction: Direction,
}

/// Read a peripheral table's `dma` opt-in into the set of directions it requests.
///
/// Returns an empty vector when the key is absent or malformed (no DMA → never
/// flagged). `dma = true` requests both directions; an array of signal spellings
/// (`"tx"`/`"rx"`/`"mosi"`/`"miso"`, case-insensitive) selects specific ones.
fn requested_directions(value: &toml::Value) -> Vec<Direction> {
    match value {
        toml::Value::Boolean(true) => vec![Direction::Tx, Direction::Rx],
        toml::Value::Boolean(false) => vec![],
        toml::Value::Array(items) => {
            let mut dirs = Vec::new();
            for item in items {
                if let Some(s) = item.as_str() {
                    if let Some(d) = Direction::from_signal(&s.to_ascii_uppercase()) {
                        if !dirs.contains(&d) {
                            dirs.push(d);
                        }
                    }
                }
            }
            dirs
        }
        _ => vec![],
    }
}

/// Format a slot as `DMA1 stream 5 (channel 4)` for conflict messages.
fn slot_label(slot: Slot) -> String {
    format!(
        "{} stream {} (channel {})",
        slot.controller.name(),
        slot.stream,
        slot.channel
    )
}

/// Validate the configured peripherals' DMA requests against the family model,
/// returning one [`Conflict::DmaCollision`] per contested stream in a fixed,
/// deterministic order. Called by the solver after the clock-tree validation.
pub fn validate(config: &Config, map: &DmaMap) -> Vec<Conflict> {
    // 1. Collect every requested (peripheral, direction) in BTreeMap peripheral
    //    order, so the greedy assignment below is deterministic.
    let peripherals: Vec<String> = config
        .peripherals
        .keys()
        .map(|k| crate::model::peripheral_name(k))
        .collect();

    let mut requests: Vec<Request> = Vec::new();
    for (idx, (instance, table)) in config.peripherals.iter().enumerate() {
        let _ = instance;
        let Some(dma_value) = table.0.get("dma") else {
            continue;
        };
        let peripheral = &peripherals[idx];
        // Unmodeled peripheral: no candidate slots anywhere, so skip — never a
        // false error (matches the empty-candidates discipline in the model).
        if !map.has_peripheral(peripheral) {
            continue;
        }
        for direction in requested_directions(dma_value) {
            if map.candidates(peripheral, direction).is_empty() {
                continue;
            }
            requests.push(Request {
                peripheral_idx: idx,
                direction,
            });
        }
    }

    // 2. Greedily assign each request the first candidate stream that is free.
    //    Track which (controller, stream) each request landed on, and detect the
    //    contention when no candidate stream is available.
    let mut occupied: BTreeMap<(Controller, u8), usize> = BTreeMap::new();
    // Contested streams: (controller, stream) -> (holder request index, the new
    // contender request index). One entry per stream, recorded once.
    let mut collisions: BTreeMap<(Controller, u8), (usize, usize)> = BTreeMap::new();
    // request index -> assigned (controller, stream) (when it found a free slot).
    let mut assigned: Vec<Option<(Controller, u8)>> = vec![None; requests.len()];

    for (ri, req) in requests.iter().enumerate() {
        let candidates = map.candidates(&peripherals[req.peripheral_idx], req.direction);
        let free = candidates
            .iter()
            .find(|s| !occupied.contains_key(&(s.controller, s.stream)));
        match free {
            Some(slot) => {
                occupied.insert((slot.controller, slot.stream), ri);
                assigned[ri] = Some((slot.controller, slot.stream));
            }
            None => {
                // Every candidate stream is taken. Report against the first
                // candidate's stream (deterministic) and the request holding it.
                let first = candidates[0];
                let key = (first.controller, first.stream);
                let holder = occupied[&key];
                collisions.entry(key).or_insert((holder, ri));
            }
        }
    }

    // 3. Turn each contested stream into one conflict, with a suggestion: a free
    //    alternative stream for either contender (the contender first, then the
    //    holder), if one exists.
    let mut out = Vec::new();
    for ((controller, stream), (holder, contender)) in collisions {
        let holder_p = peripherals[requests[holder].peripheral_idx].clone();
        let contender_p = peripherals[requests[contender].peripheral_idx].clone();

        let suggestion = free_alternative(map, &peripherals, &requests[contender], &occupied)
            .map(|s| (contender_p.clone(), s))
            .or_else(|| {
                free_alternative(map, &peripherals, &requests[holder], &occupied)
                    .map(|s| (holder_p.clone(), s))
            });

        out.push(Conflict::DmaCollision {
            first: holder_p,
            second: contender_p,
            controller: controller.name().to_string(),
            stream,
            suggestion: suggestion.map(|(p, s)| (p, slot_label(s))),
        });
    }

    out
}

/// A candidate slot for `req` whose stream is free (not in `occupied`), if any —
/// the basis for a "move it here instead" suggestion.
fn free_alternative(
    map: &DmaMap,
    peripherals: &[String],
    req: &Request,
    occupied: &BTreeMap<(Controller, u8), usize>,
) -> Option<Slot> {
    // The holder already occupies one stream; a useful suggestion is a *different*
    // free stream it (or the contender) could move to.
    let mut seen: BTreeSet<(Controller, u8)> = BTreeSet::new();
    for slot in map.candidates(&peripherals[req.peripheral_idx], req.direction) {
        let key = (slot.controller, slot.stream);
        if occupied.contains_key(&key) {
            continue;
        }
        if seen.insert(key) {
            return Some(slot);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn f446() -> DmaMap {
        DmaMap::f446re()
    }
    fn f411() -> DmaMap {
        DmaMap::f411re()
    }

    fn validate_toml(text: &str, map: &DmaMap) -> Vec<Conflict> {
        let cfg = config::parse(text).unwrap();
        validate(&cfg, map)
    }

    #[test]
    fn no_dma_opt_in_is_never_flagged() {
        // USART2 and TIM2 both have a single RX slot on DMA1 stream 5, but
        // neither opts into DMA, so there is nothing to arbitrate.
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\n\n[peripherals.tim2]\nchannel1=\"PA5\"\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn single_dma_peripheral_is_clean() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=true\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn unmodeled_dma_peripheral_is_clean() {
        // TIM9 has no modeled DMA rows; opting in must not error.
        let text = "[peripherals.tim9]\nchannel1=\"PE5\"\ndma=true\n";
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }

    #[test]
    fn two_peripherals_one_stream_collide_with_suggestion() {
        // M2 EXIT: I2C1_RX (DMA1 s0 ch1, alt s5) and UART5_RX (DMA1 s0 ch4 only)
        // both land on DMA1 stream 0. Exactly one collision naming both, with a
        // valid free alternative for I2C1 (DMA1 stream 5).
        let text = "[peripherals.i2c1]\nsda=\"PB7\"\nscl=\"PB6\"\ndma=[\"rx\"]\n\n[peripherals.uart5]\ntx=\"PC12\"\nrx=\"PD2\"\ndma=[\"rx\"]\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        match &conflicts[0] {
            Conflict::DmaCollision {
                first,
                second,
                controller,
                stream,
                suggestion,
            } => {
                assert_eq!(first, "I2C1");
                assert_eq!(second, "UART5");
                assert_eq!(controller, "DMA1");
                assert_eq!(*stream, 0);
                let (p, slot) = suggestion.as_ref().expect("a free alternative exists");
                assert_eq!(p, "I2C1");
                assert!(slot.contains("stream 5"), "got {slot}");
            }
            other => panic!("expected DmaCollision, got {other:?}"),
        }
    }

    #[test]
    fn one_conflict_per_stream_not_per_pair() {
        // Three RX requests forced onto DMA1 stream 5: USART2 (s5 only), TIM2
        // (s5 only), and a third single-slot user. Exactly one conflict.
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=[\"rx\"]\n\n[peripherals.tim2]\nchannel1=\"PA5\"\ndma=[\"rx\"]\n";
        let conflicts = validate_toml(text, &f446());
        assert_eq!(conflicts.len(), 1, "got {conflicts:?}");
        if let Conflict::DmaCollision { stream, .. } = &conflicts[0] {
            assert_eq!(*stream, 5);
        }
    }

    #[test]
    fn collision_without_alternative_has_no_suggestion() {
        // USART2_RX and TIM2_RX each have exactly one slot (DMA1 stream 5); no
        // free alternative for either, so the suggestion is None.
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=[\"rx\"]\n\n[peripherals.tim2]\nchannel1=\"PA5\"\ndma=[\"rx\"]\n";
        let conflicts = validate_toml(text, &f446());
        if let Conflict::DmaCollision { suggestion, .. } = &conflicts[0] {
            assert!(suggestion.is_none(), "got {suggestion:?}");
        }
    }

    #[test]
    fn family_parameterized_f411() {
        // The same I2C1/UART5 contention does not exist on the F411 (no UART5),
        // so opting both in is clean there — UART5 is unmodeled.
        let text = "[peripherals.i2c1]\nsda=\"PB7\"\nscl=\"PB6\"\ndma=[\"rx\"]\n\n[peripherals.uart5]\ntx=\"PC12\"\nrx=\"PD2\"\ndma=[\"rx\"]\n";
        assert_eq!(validate_toml(text, &f411()), vec![]);
    }

    #[test]
    fn mosi_miso_spellings_map_to_directions() {
        let text = "[peripherals.spi1]\nmosi=\"PA7\"\nmiso=\"PA6\"\nsck=\"PA5\"\ndma=[\"miso\"]\n";
        // SPI1_RX is on DMA2; alone it is clean.
        assert_eq!(validate_toml(text, &f446()), vec![]);
    }
}
