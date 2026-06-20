`captured_swo.bin` is a real capture (`01 4f 01 4b`) — `blink_itm_hw.bin`
flashed to a NUCLEO-F411RE, OpenOCD's TPIU/SWO trace server on port 3344,
captured immediately after `reset run`. Matches the wire format the
fabricated placeholder it replaced predicted: two single-byte ITM SWIT
packets on stimulus port 0, payload "OK".

Getting a real capture exposed two gaps the fabricated bytes hid:
1. The hardware variant didn't enable SWO at all — PB3 needs its TRACESWO
   alternate function (AF0) and `DBGMCU_CR.TRACE_IOEN` set, both off after
   reset by default. QEMU has no such pin-mux/DBGMCU gating, so this was
   invisible there. Fixed in `blink_itm.c`'s `enable_swo_pin()`.
2. `ITM_STIM0` was typed `volatile uint32_t*`; storing a `char` through it
   still issued a 32-bit bus access, which real ITM hardware packetizes as a
   4-byte SWIT packet (`03 4f 00 00 00 ...`), not the expected 1-byte one.
   Fixed by typing the stimulus port pointer `uint8_t*` instead.

`tests/e2e_hardware_replay.rs` doesn't care about capture provenance, only
that the bytes decode to the same `ItmEvent` shape as the QEMU leg.
