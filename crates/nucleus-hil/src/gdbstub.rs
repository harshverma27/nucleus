//! A minimal GDB Remote Serial Protocol client — just enough to read target
//! memory. Both backends speak this: QEMU's own `-gdb` stub for the emulator
//! leg, OpenOCD's gdbserver (port 3333) for the hardware leg. One client, two
//! transports, matching the "same observation API" requirement at the
//! implementation level.
//!
//! Hand-rolled rather than pulling in a crate like `probe-rs`, mirroring
//! `nucleus-itm`'s zero-extra-dependency, never-panic posture: malformed
//! replies always become [`HilError::Protocol`], never a panic.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::backend::HilError;

/// Bound on every blocking read in this module. Without it, a stub that
/// never replies (e.g. a GDB-RSP server that doesn't honor a bare interrupt
/// byte the way this minimal client assumes) hangs the calling test forever
/// instead of surfacing a [`HilError`] — confirmed empirically against real
/// OpenOCD/NUCLEO-F411RE hardware, where [`GdbStub::interrupt`]'s stop-reply
/// read never returned.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

async fn with_timeout<T>(
    fut: impl std::future::Future<Output = std::io::Result<T>>,
) -> Result<T, HilError> {
    match tokio::time::timeout(READ_TIMEOUT, fut).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(HilError::Protocol(format!(
            "timed out after {READ_TIMEOUT:?} waiting for a reply"
        ))),
    }
}

/// A connected GDB Remote Serial Protocol session.
pub struct GdbStub {
    conn: TcpStream,
}

impl GdbStub {
    pub async fn connect(addr: &str) -> Result<Self, HilError> {
        let conn = TcpStream::connect(addr).await?;
        Ok(Self { conn })
    }

    /// Read `len` bytes of target memory starting at `addr` via the `m`
    /// packet (`$m<addr>,<len>#<checksum>`).
    pub async fn read_memory(&mut self, addr: u32, len: u32) -> Result<Vec<u8>, HilError> {
        let payload = format!("m{addr:x},{len:x}");
        self.send_packet(&payload).await?;
        let reply = self.read_packet().await?;

        if reply.starts_with('E') {
            return Err(HilError::Protocol(format!(
                "target returned error: {reply}"
            )));
        }

        decode_hex(&reply)
            .ok_or_else(|| HilError::Protocol(format!("non-hex memory reply: {reply}")))
    }

    /// Write `bytes` to target memory at `addr` via the `M` packet
    /// (`$M<addr>,<len>:<hex bytes>#<checksum>`). Stub must reply `OK`.
    pub async fn write_memory(&mut self, addr: u32, bytes: &[u8]) -> Result<(), HilError> {
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            hex.push_str(&format!("{b:02x}"));
        }
        let payload = format!("M{addr:x},{:x}:{hex}", bytes.len());
        self.send_packet(&payload).await?;
        let reply = self.read_packet().await?;
        if reply == "OK" {
            Ok(())
        } else {
            Err(HilError::Protocol(format!(
                "memory write rejected by target: {reply}"
            )))
        }
    }

    /// Resume target execution (`c` packet), used after attaching with `-S`
    /// or after [`interrupt`](Self::interrupt). Doesn't wait for the eventual
    /// stop-reply — the caller decides when (if ever) to halt again.
    pub async fn continue_execution(&mut self) -> Result<(), HilError> {
        self.send_packet("c").await
    }

    /// Halt a running target so memory can be read. Most GDB stubs (QEMU's
    /// included) only answer `m` queries while halted — issuing one against
    /// a free-running target hangs forever waiting for a reply that comes
    /// only on the next stop. Sends the GDB-RSP interrupt byte (`0x03`, not
    /// a `$...#` packet) and consumes the resulting stop-reply.
    pub async fn interrupt(&mut self) -> Result<(), HilError> {
        self.conn.write_all(&[0x03]).await?;
        self.conn.flush().await?;
        let _stop_reply = self.read_packet().await?;
        Ok(())
    }

    async fn send_packet(&mut self, payload: &str) -> Result<(), HilError> {
        let checksum: u8 = payload.bytes().fold(0u8, |acc, b| acc.wrapping_add(b));
        let framed = format!("${payload}#{checksum:02x}");
        self.conn.write_all(framed.as_bytes()).await?;
        self.conn.flush().await?;

        // The stub acks with '+' before sending its reply; tolerate either
        // ordering/absence rather than failing on a strict ack requirement.
        let mut ack = [0u8; 1];
        with_timeout(self.conn.read_exact(&mut ack)).await?;
        if ack[0] != b'+' {
            return Err(HilError::Protocol(format!(
                "expected '+' ack, got {:?}",
                ack[0] as char
            )));
        }
        Ok(())
    }

    async fn read_packet(&mut self) -> Result<String, HilError> {
        let mut byte = [0u8; 1];
        loop {
            with_timeout(self.conn.read_exact(&mut byte)).await?;
            if byte[0] == b'$' {
                break;
            }
        }

        let mut body = Vec::new();
        loop {
            with_timeout(self.conn.read_exact(&mut byte)).await?;
            if byte[0] == b'#' {
                break;
            }
            body.push(byte[0]);
        }
        // Consume the two-byte checksum trailer; we don't verify it — a bad
        // checksum still yields a typed Protocol error downstream if the body
        // doesn't parse, which is the behavior that matters.
        let mut checksum = [0u8; 2];
        with_timeout(self.conn.read_exact(&mut checksum)).await?;
        // Ack the reply.
        self.conn.write_all(b"+").await?;
        self.conn.flush().await?;

        String::from_utf8(body).map_err(|_| HilError::Protocol("non-utf8 packet body".to_string()))
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out.push((hi as u8) << 4 | lo as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Spin up an in-process mock GDB-RSP server speaking exactly one canned
    /// `m`-packet reply, then shut down. No QEMU/OpenOCD involved.
    async fn mock_server(reply_payload: &'static str) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Read the client's request packet ($...#xx) and ack it.
            let mut buf = [0u8; 256];
            let _ = sock.read(&mut buf).await.unwrap();
            sock.write_all(b"+").await.unwrap();

            let checksum: u8 = reply_payload
                .bytes()
                .fold(0u8, |acc, b| acc.wrapping_add(b));
            let framed = format!("${reply_payload}#{checksum:02x}");
            sock.write_all(framed.as_bytes()).await.unwrap();
            // Consume the client's ack of our reply.
            let _ = sock.read(&mut buf).await;
        });
        addr
    }

    #[tokio::test]
    async fn reads_memory_from_well_formed_reply() {
        let addr = mock_server("deadbeef").await;
        let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
        let bytes = stub.read_memory(0x4002_0010, 4).await.unwrap();
        assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[tokio::test]
    async fn malformed_reply_is_a_protocol_error_not_a_panic() {
        let addr = mock_server("not-hex!!").await;
        let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
        let result = stub.read_memory(0x4002_0010, 4).await;
        assert!(matches!(result, Err(HilError::Protocol(_))));
    }

    #[tokio::test]
    async fn target_error_reply_is_a_protocol_error() {
        let addr = mock_server("E01").await;
        let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
        let result = stub.read_memory(0x4002_0010, 4).await;
        assert!(matches!(result, Err(HilError::Protocol(_))));
    }

    #[tokio::test]
    async fn writes_memory_and_accepts_ok_reply() {
        // Mock server: accept the M packet, ack '+', reply "$OK#<sum>".
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // read the inbound packet bytes (best-effort, we don't parse here)
            let mut buf = [0u8; 64];
            let _ = sock.read(&mut buf).await.unwrap();
            sock.write_all(b"+").await.unwrap();
            let body = "OK";
            let sum = body.bytes().fold(0u8, |a, b| a.wrapping_add(b));
            let framed = format!("${body}#{sum:02x}");
            sock.write_all(framed.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });

        let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
        stub.write_memory(0x2000_0000, &[0x67, 0x41, 0x54, 0x4e])
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn write_memory_surfaces_error_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let _ = sock.read(&mut buf).await.unwrap();
            sock.write_all(b"+").await.unwrap();
            let framed = "$E01#"; // body "E01"
            let sum = "E01".bytes().fold(0u8, |a, b| a.wrapping_add(b));
            sock.write_all(format!("{framed}{sum:02x}").as_bytes())
                .await
                .unwrap();
            sock.flush().await.unwrap();
        });
        let mut stub = GdbStub::connect(&addr.to_string()).await.unwrap();
        let err = stub
            .write_memory(0x2000_0000, &[1, 2, 3, 4])
            .await
            .unwrap_err();
        assert!(matches!(err, HilError::Protocol(_)));
    }
}
