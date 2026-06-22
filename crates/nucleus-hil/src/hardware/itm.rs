//! Decode the hardware backend's SWO byte stream into [`ItmEvent`]s.
//!
//! Reuses [`nucleus_trace::Source`] (`Tcp` for live OpenOCD, `File` for
//! replay/tests) and [`nucleus_itm::Decoder`] directly. This is a new, minimal
//! ingestion loop rather than a call into `nucleus_trace::server::TraceServer`
//! — that type is coupled to WebSocket fan-out and `[[trace]]` variable
//! translation that M5 doesn't need; it just maps `Packet` to [`ItmEvent`].

use nucleus_itm::{Decoder, Packet};
use nucleus_trace::Source;
use tokio::io::AsyncReadExt;

use crate::backend::{HilError, ItmEvent};

/// Read `source` to EOF (or until `max_bytes` is read, whichever first),
/// decoding every complete packet along the way. `Instrumentation` packets
/// become [`ItmEvent`]s; every other packet kind is currently dropped (M5 only
/// needs instrumentation/log events — DWT/timestamp interpretation is a
/// later milestone's concern, not a missing feature here).
pub async fn drain_events(source: &Source, max_bytes: usize) -> Result<Vec<ItmEvent>, HilError> {
    let mut reader = source.open().await?;
    let mut decoder = Decoder::new();
    let mut events = Vec::new();
    let mut total_read = 0usize;
    let mut chunk = [0u8; 4096];

    loop {
        if total_read >= max_bytes {
            break;
        }
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        total_read += n;
        for packet in decoder.decode(&chunk[..n]) {
            if let Packet::Instrumentation { port, data } = packet {
                events.push(ItmEvent { port, data });
            }
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Hand-encode one ITM instrumentation (SWIT) packet for port 0 carrying
    /// `byte` as a 1-byte payload, per ARMv7-M DDI 0403E §C1.10: header bit
    /// pattern `SS0 01 SH` with size=00 (1 byte), SH=0 (software source).
    fn encode_swit_packet(port: u8, byte: u8) -> Vec<u8> {
        let header = (port << 3) | 0b001;
        vec![header, byte]
    }

    /// A scratch file under the OS temp dir, removed on drop. Avoids pulling
    /// in a `tempfile` dependency for two tests.
    struct ScratchFile(PathBuf);

    impl ScratchFile {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("nucleus-hil-{name}-{nanos}"));
            Self(path)
        }

        fn write(&self, bytes: &[u8]) {
            let mut file = std::fs::File::create(&self.0).unwrap();
            file.write_all(bytes).unwrap();
            file.flush().unwrap();
        }
    }

    impl Drop for ScratchFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[tokio::test]
    async fn decodes_instrumentation_packets_from_a_file_source() {
        let scratch = ScratchFile::new("decode-test");
        let mut bytes = encode_swit_packet(0, b'h');
        bytes.extend(encode_swit_packet(0, b'i'));
        scratch.write(&bytes);

        let source = Source::File(scratch.0.clone());
        let events = drain_events(&source, 1 << 16).await.unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].port, 0);
        assert_eq!(events[0].data, vec![b'h']);
        assert_eq!(events[1].data, vec![b'i']);
    }

    #[tokio::test]
    async fn empty_source_yields_no_events() {
        let scratch = ScratchFile::new("empty-test");
        scratch.write(b"");
        let source = Source::File(scratch.0.clone());
        let events = drain_events(&source, 1 << 16).await.unwrap();
        assert!(events.is_empty());
    }
}
