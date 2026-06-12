//! A hand-rolled ARM CoreSight **ITM/SWO** packet decoder.
//!
//! The SWO pin emits a byte stream of ITM and DWT packets (ARMv7-M Architecture
//! Reference Manual, DDI 0403E, §C1.10). This crate turns that byte stream into
//! structured [`Packet`]s.
//!
//! Design constraints from the project README (all load-bearing):
//!
//! - **Streaming.** [`Decoder::decode`] is fed arbitrary byte chunks; packets
//!   may span chunk boundaries. Incomplete packets are buffered until the rest
//!   arrives.
//! - **Never panics.** Hostile or corrupt input must not crash the daemon. The
//!   decoder is `#![forbid(unsafe_code)]` (via the workspace lint) and every
//!   slice access is length-checked; an unrecognized header byte is skipped to
//!   resync rather than aborting.
//! - **Resynchronizes.** Synchronization packets re-align the stream after a
//!   dropped connection, and the internal buffer is capped so a stuck stream
//!   can't grow memory without bound.
//! - **Zero dependencies** on the parsing path, so the whole thing is auditable.
//!
//! What the higher layer ([`nucleus-trace`](../nucleus_trace/index.html)) cares
//! about is [`Packet::Instrumentation`] — the SWIT software-source packets that
//! carry `printf`-style logs (port 0) and traced variables (ports 1–7). The
//! decoder is config-agnostic: it reports the raw port and payload bytes and
//! leaves type/name mapping to the caller.

/// A decoded ITM/SWO packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// Software-source (SWIT) packet: instrumentation written by firmware to a
    /// stimulus `port` (0–31). `data` is 1, 2, or 4 bytes (the access size).
    Instrumentation { port: u8, data: Vec<u8> },
    /// Hardware-source (DWT) packet with a discriminator id and payload.
    Hardware { discriminator: u8, data: Vec<u8> },
    /// Trace overflow: the device's ITM FIFO filled and data was lost.
    Overflow,
    /// Synchronization packet — a long run of zero bits used to align the stream.
    Synchronization,
    /// Local timestamp (delta time since the previous timestamp).
    LocalTimestamp { tc: u8, ts: u32 },
    /// Global (absolute) timestamp.
    GlobalTimestamp { ts: u64 },
    /// Extension packet (e.g. stimulus-port page). Reported but not interpreted.
    Extension { data: Vec<u8> },
}

/// Upper bound on the internal buffer. If a malformed/stuck stream grows the
/// buffer past this, it is dropped and decoding resynchronizes on the next
/// synchronization packet — bounding memory use to O(1).
const MAX_BUFFER: usize = 1 << 20;

/// The maximum payload bytes a continuation-style protocol packet (timestamps,
/// extension) may carry. The spec caps these well below this.
const MAX_CONTINUATION: usize = 4;

/// A streaming ITM/SWO decoder. Feed it bytes with [`Decoder::decode`].
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

/// The result of attempting to parse one packet from the front of a slice.
enum Step {
    /// Not enough bytes yet for a complete packet; wait for more.
    NeedMore,
    /// Consumed `consumed` bytes, optionally yielding a packet (protocol packets
    /// like sync padding may consume bytes without producing user output).
    Consumed {
        consumed: usize,
        packet: Option<Packet>,
    },
}

impl Decoder {
    /// A fresh decoder with an empty buffer.
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Feed a chunk of SWO bytes and return every complete packet it completes.
    /// Bytes that form only part of a packet are retained for the next call.
    pub fn decode(&mut self, bytes: &[u8]) -> Vec<Packet> {
        self.buffer.extend_from_slice(bytes);

        let mut packets = Vec::new();
        let mut pos = 0;
        loop {
            match parse_one(&self.buffer[pos..]) {
                Step::NeedMore => break,
                Step::Consumed { consumed, packet } => {
                    // `parse_one` never reports zero progress, but guard anyway
                    // so a logic slip can never spin forever.
                    if consumed == 0 {
                        break;
                    }
                    pos += consumed;
                    if let Some(p) = packet {
                        packets.push(p);
                    }
                }
            }
        }

        // Drop the consumed prefix; keep the unparsed tail for next time.
        self.buffer.drain(..pos);

        // Safety valve: never let the buffer grow without bound. Dropping it
        // costs at most one packet; the stream realigns on the next sync.
        if self.buffer.len() > MAX_BUFFER {
            self.buffer.clear();
        }

        packets
    }

    /// Discard any buffered partial packet (e.g. after a connection drop). The
    /// next synchronization packet re-aligns the stream.
    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

/// Try to parse a single packet from the front of `buf`.
fn parse_one(buf: &[u8]) -> Step {
    let Some(&header) = buf.first() else {
        return Step::NeedMore;
    };

    // Source packet: the low two bits encode a non-zero payload size.
    let size_code = header & 0b11;
    if size_code != 0 {
        let size = match size_code {
            0b01 => 1,
            0b10 => 2,
            _ => 4, // 0b11
        };
        let total = 1 + size;
        if buf.len() < total {
            return Step::NeedMore;
        }
        let data = buf[1..total].to_vec();
        let id = header >> 3; // bits[7:3]
        let packet = if header & 0b100 == 0 {
            Packet::Instrumentation { port: id, data }
        } else {
            Packet::Hardware {
                discriminator: id,
                data,
            }
        };
        return Step::Consumed {
            consumed: total,
            packet: Some(packet),
        };
    }

    // Otherwise it's a protocol packet (low two bits are zero).
    match header {
        0x00 => parse_sync(buf),
        0x70 => one(Packet::Overflow),
        _ => parse_protocol(buf, header),
    }
}

/// Synchronization packet: a run of `0x00` bytes terminated by `0x80`.
fn parse_sync(buf: &[u8]) -> Step {
    let zeros = buf.iter().take_while(|&&b| b == 0x00).count();
    if zeros == buf.len() {
        // All zeros so far — wait for the terminator (bounded by MAX_BUFFER).
        return Step::NeedMore;
    }
    if buf[zeros] == 0x80 {
        Step::Consumed {
            consumed: zeros + 1,
            packet: Some(Packet::Synchronization),
        }
    } else {
        // Zeros not terminated by 0x80: treat the run as sync padding and
        // resume parsing at the next byte (it will be classified on its own).
        Step::Consumed {
            consumed: zeros,
            packet: Some(Packet::Synchronization),
        }
    }
}

/// Protocol packets other than sync/overflow: timestamps, extension, reserved.
fn parse_protocol(buf: &[u8], header: u8) -> Step {
    // Local timestamp, format 2 (single byte): `0b0TTT_0000`, TTT != 0.
    if header & 0b1000_1111 == 0 {
        return Step::Consumed {
            consumed: 1,
            packet: Some(Packet::LocalTimestamp {
                tc: 0,
                ts: u32::from((header >> 4) & 0b111),
            }),
        };
    }

    // Local timestamp, format 1: `0b11xx_0000`, with continuation payload bytes.
    if header & 0b1100_1111 == 0b1100_0000 {
        let tc = (header >> 4) & 0b11;
        return continuation(buf, |payload| Packet::LocalTimestamp {
            tc,
            ts: fold_le7(payload) as u32,
        });
    }

    // Global timestamp 1 (0x94) and 2 (0xB4): continuation payload bytes.
    if header == 0x94 || header == 0xB4 {
        return continuation(buf, |payload| Packet::GlobalTimestamp {
            ts: fold_le7(payload),
        });
    }

    // Extension packet: bit[3] set (bits[2:0] = 0b000).
    if header & 0b0000_1011 == 0b0000_1000 {
        return continuation(buf, |payload| Packet::Extension {
            data: payload.to_vec(),
        });
    }

    // Reserved / unrecognized protocol header: skip one byte to resync.
    Step::Consumed {
        consumed: 1,
        packet: None,
    }
}

/// Consume a continuation-style protocol packet: a header byte followed, if the
/// header's continuation bit (bit 7) is set, by up to [`MAX_CONTINUATION`]
/// payload bytes, each of which sets bit 7 to request another.
fn continuation(buf: &[u8], make: impl FnOnce(&[u8]) -> Packet) -> Step {
    // buf[0] is the header (guaranteed present by the caller).
    if buf[0] & 0x80 == 0 {
        return Step::Consumed {
            consumed: 1,
            packet: Some(make(&[])),
        };
    }
    let mut i = 1;
    let mut count = 0;
    loop {
        let Some(&b) = buf.get(i) else {
            return Step::NeedMore;
        };
        i += 1;
        count += 1;
        if b & 0x80 == 0 || count >= MAX_CONTINUATION {
            break;
        }
    }
    Step::Consumed {
        consumed: i,
        packet: Some(make(&buf[1..i])),
    }
}

/// Fold little-endian 7-bit continuation groups into an integer.
fn fold_le7(payload: &[u8]) -> u64 {
    let mut value = 0u64;
    for (k, b) in payload.iter().enumerate() {
        value |= u64::from(b & 0x7F) << (7 * k as u32);
    }
    value
}

fn one(packet: Packet) -> Step {
    Step::Consumed {
        consumed: 1,
        packet: Some(packet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode a whole buffer in one shot.
    fn decode_all(bytes: &[u8]) -> Vec<Packet> {
        Decoder::new().decode(bytes)
    }

    #[test]
    fn decodes_port0_log_bytes() {
        // Port 0, 1-byte payload: header = (0 << 3) | 0b01 = 0x01.
        let stream = [0x01, b'H', 0x01, b'i'];
        assert_eq!(
            decode_all(&stream),
            vec![
                Packet::Instrumentation {
                    port: 0,
                    data: vec![b'H']
                },
                Packet::Instrumentation {
                    port: 0,
                    data: vec![b'i']
                },
            ]
        );
    }

    #[test]
    fn decodes_4byte_variable_on_port1() {
        // Port 1, 4-byte payload: header = (1 << 3) | 0b11 = 0x0B.
        let value = 3.5f32.to_le_bytes();
        let mut stream = vec![0x0B];
        stream.extend_from_slice(&value);
        assert_eq!(
            decode_all(&stream),
            vec![Packet::Instrumentation {
                port: 1,
                data: value.to_vec()
            }]
        );
    }

    #[test]
    fn distinguishes_hardware_source_packets() {
        // Hardware bit (0b100) set: header = (2 << 3) | 0b100 | 0b01 = 0x15.
        assert_eq!(
            decode_all(&[0x15, 0x42]),
            vec![Packet::Hardware {
                discriminator: 2,
                data: vec![0x42]
            }]
        );
    }

    #[test]
    fn decodes_overflow_and_sync() {
        assert_eq!(decode_all(&[0x70]), vec![Packet::Overflow]);
        // Sync: five zero bytes then 0x80.
        assert_eq!(
            decode_all(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x80]),
            vec![Packet::Synchronization]
        );
    }

    #[test]
    fn handles_packets_split_across_reads() {
        // A 4-byte payload packet delivered one byte at a time must still decode
        // exactly once, when the last byte arrives.
        let mut dec = Decoder::new();
        let stream = [0x0B, 0xAA, 0xBB, 0xCC, 0xDD];
        let mut out = Vec::new();
        for b in stream {
            out.extend(dec.decode(&[b]));
        }
        assert_eq!(
            out,
            vec![Packet::Instrumentation {
                port: 1,
                data: vec![0xAA, 0xBB, 0xCC, 0xDD]
            }]
        );
    }

    #[test]
    fn chunking_does_not_change_the_decoded_stream() {
        // Decoding a fixed buffer all at once must equal decoding it in pieces.
        let stream = [
            0x01, b'A', 0x70, 0x0B, 1, 2, 3, 4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x09, 0xDE,
            0xAD,
        ];
        let whole = decode_all(&stream);
        for chunk in 1..=stream.len() {
            let mut dec = Decoder::new();
            let mut pieced = Vec::new();
            for part in stream.chunks(chunk) {
                pieced.extend(dec.decode(part));
            }
            assert_eq!(whole, pieced, "mismatch at chunk size {chunk}");
        }
    }

    #[test]
    fn local_timestamp_single_and_multibyte() {
        // Format 2 single byte: 0b0011_0000 = 0x30 (TTT = 011).
        assert_eq!(
            decode_all(&[0x30]),
            vec![Packet::LocalTimestamp { tc: 0, ts: 3 }]
        );
        // Format 1: header 0xC0 + two continuation payload bytes.
        assert_eq!(
            decode_all(&[0xC0, 0x81, 0x02]),
            vec![Packet::LocalTimestamp {
                tc: 0,
                ts: 0x01 | (0x02 << 7)
            }]
        );
    }

    #[test]
    fn resyncs_after_garbage_via_sync_packet() {
        // A reserved protocol byte (0x04) is skipped; a following source packet
        // still decodes.
        let out = decode_all(&[0x04, 0x01, b'Z']);
        assert_eq!(
            out,
            vec![Packet::Instrumentation {
                port: 0,
                data: vec![b'Z']
            }]
        );
    }

    /// Deterministic xorshift PRNG so the fuzz test is reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn byte(&mut self) -> u8 {
            (self.next() & 0xFF) as u8
        }
    }

    #[test]
    fn never_panics_on_random_input() {
        // The core robustness requirement: arbitrary bytes, delivered in
        // arbitrary chunk sizes, must never panic and must stay memory-bounded.
        let mut rng = Rng(0x1234_5678_9abc_def0);
        for _ in 0..2_000 {
            let len = (rng.next() % 512) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();

            let mut dec = Decoder::new();
            let mut single = Vec::new();
            for &b in &bytes {
                single.extend(dec.decode(&[b]));
            }
            // Feeding the same bytes in random chunks yields the same packets.
            let mut dec2 = Decoder::new();
            let mut chunked = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                let step = ((rng.next() % 7) + 1) as usize;
                let end = (i + step).min(bytes.len());
                chunked.extend(dec2.decode(&bytes[i..end]));
                i = end;
            }
            assert_eq!(single, chunked);
        }
    }

    #[test]
    fn long_zero_run_stays_memory_bounded() {
        // A pathological all-zero stream must not grow the buffer unboundedly.
        let mut dec = Decoder::new();
        for _ in 0..100 {
            dec.decode(&[0u8; 50_000]);
        }
        // No assertion on packets; the point is that this returns without OOM
        // or panic. (Buffer is internally capped at MAX_BUFFER.)
    }
}
