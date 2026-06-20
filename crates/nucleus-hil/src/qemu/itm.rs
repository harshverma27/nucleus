//! Decode the QEMU backend's "ITM" stream.
//!
//! QEMU's STM32F4 SoC model has no TPIU/ITM block (confirmed against QEMU
//! 11.0's `hw/arm/stm32f4xx_soc.c` — it isn't modeled). Workaround: the
//! `blink_itm_qemu` firmware variant writes already-ITM-encoded bytes over an
//! ARM semihosting channel instead of a real stimulus-port write, so the
//! bytes on the wire are bit-for-bit what a real ITM peripheral would have
//! produced. QEMU is invoked with
//! `-semihosting-config enable=on,target=native,chardev=hilsemi
//!  -chardev file,id=hilsemi,path=<path>` (a plain output file, not a named
//! pipe — QEMU's `pipe` chardev backend didn't reliably deliver bytes in
//! testing on this host; `file` is simpler and confirmed working: it flushes
//! semihosting writes as they happen), and this module reads that path and
//! feeds the bytes through the **same** [`nucleus_itm::Decoder`] the
//! hardware leg uses — the decode path is genuinely shared between backends,
//! only the byte *origin* differs. This is a real fidelity gap (not a
//! faithful SWO emulation); document it again wherever M10's lockstep
//! comparator reasons about ITM events.

use nucleus_itm::{Decoder, Packet};
use tokio::io::AsyncReadExt;

use crate::backend::{HilError, ItmEvent};

/// Read the semihosting-channel fifo at `path` until `max_bytes` is consumed
/// or the writer closes it, decoding every complete ITM packet along the way.
pub async fn drain_events(
    path: &std::path::Path,
    max_bytes: usize,
) -> Result<Vec<ItmEvent>, HilError> {
    let mut reader = tokio::fs::File::open(path).await?;
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

    struct ScratchFile(PathBuf);

    impl ScratchFile {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("nucleus-hil-qemu-{name}-{nanos}"));
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

    /// Same SWIT encoding `hardware::itm`'s tests use — proves both backends'
    /// decode paths agree on the wire format.
    fn encode_swit_packet(port: u8, byte: u8) -> Vec<u8> {
        vec![(port << 3) | 0b001, byte]
    }

    #[tokio::test]
    async fn decodes_pre_encoded_itm_bytes_from_the_semihosting_channel() {
        let scratch = ScratchFile::new("itm-decode");
        scratch.write(&encode_swit_packet(0, b'q'));

        let events = drain_events(&scratch.0, 1 << 16).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].port, 0);
        assert_eq!(events[0].data, vec![b'q']);
    }
}
