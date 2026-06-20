`captured_swo.bin` is currently a **fabricated placeholder**, not a real
hardware capture: hand-crafted bytes (`01 4f 01 4b`) matching the ITM SWIT
wire format for two single-byte packets on stimulus port 0, payload "OK" —
the same log `blink_itm` emits. Swap in a real OpenOCD/ST-Link capture from
actual F446RE/F411RE hardware once available; `tests/e2e_hardware_replay.rs`
doesn't care which, only that the bytes decode to the same `ItmEvent` shape.
