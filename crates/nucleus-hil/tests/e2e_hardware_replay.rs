//! Proves the hardware backend's decode leg without a live board: feeds a
//! captured (currently fabricated, see `captured_swo.bin`'s README) SWO byte
//! stream through the same `Source::File` + decode pipeline the real OpenOCD
//! leg uses, asserting the same `ItmEvent` shape `tests/e2e_qemu.rs` proves
//! on the QEMU leg. Per the plan, the request/response half (GDB-RSP memory
//! reads over OpenOCD's gdbserver) isn't replayable as a passive byte
//! fixture — that's covered separately by `gdbstub`'s mock-server unit
//! tests.

use std::path::PathBuf;

use nucleus_hil::hardware::itm::drain_events;
use nucleus_trace::Source;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/blink_itm/captured_swo.bin")
}

#[tokio::test]
async fn decodes_captured_swo_bytes_without_a_live_board() {
    let source = Source::File(fixture_path());
    let events = drain_events(&source, 1 << 16)
        .await
        .expect("decode captured_swo.bin");

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].port, 0);
    assert_eq!(events[0].data, vec![b'O']);
    assert_eq!(events[1].data, vec![b'K']);
}
