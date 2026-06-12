# nucleus-trace

The trace backend: decode pipeline + WebSocket fan-out.

## Status (Phase 5 — complete)

```text
OpenOCD (SWO) ──▶ source ──▶ Decoder ──▶ Translator ──▶ JSON ──▶ WebSocket :7878
```

- `translate` — **pure, unit-tested** meaning assignment: stimulus port 0 →
  reassembled UTF-8 log lines (replaces `printf`); ports 1–7 → typed values
  (`f32`/`u16`/`u32`/`i32`) named by `[[trace.variables]]`. Emits `TraceEvent`s
  that serialize to JSON (`{"kind":"log"|"variable"|"overflow", …}`).
- `server` — `TraceServer` owns the decoder + translator, and broadcasts each
  event as JSON to every connected WebSocket client (tokio-tungstenite). A slow
  client drops messages rather than stalling the pipeline.
- `source` — SWO bytes from an OpenOCD TCP trace port
  (`tpiu config internal :PORT …`) or a captured-file replay, plus a best-effort
  OpenOCD telnet setup helper.
- `run_blocking(TraceOptions)` — the synchronous entry point `nucleus trace`
  calls.

An integration test (`tests/websocket.rs`) connects a real WebSocket client and
asserts it receives the decoded `log`/`variable` JSON — the end-to-end Phase 5
exit criterion. Capturing from a physical NUCLEO-F446RE additionally requires
OpenOCD + the ST-Link; the SWO command sequence is version-dependent (see
`source::openocd_enable`).
