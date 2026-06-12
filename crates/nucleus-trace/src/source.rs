//! Byte sources for the SWO trace stream, and OpenOCD setup over telnet.
//!
//! The decoder doesn't care where bytes come from. In practice they arrive from
//! OpenOCD: it captures SWO from the ST-Link and, when configured with
//! `tpiu config internal :<port> …`, streams the raw trace to a TCP socket. We
//! connect to that socket. For development and tests there is also a file
//! replay of a captured stream.
//!
//! Per the README, OpenOCD's SWO command sequence is version- and probe-
//! dependent; [`openocd_enable`] sends a best-effort sequence and the exact
//! commands are documented for the user to adapt.

use std::path::PathBuf;

use tokio::io::{AsyncRead, AsyncWriteExt};
use tokio::net::TcpStream;

/// Where to read the raw SWO byte stream from.
#[derive(Debug, Clone)]
pub enum Source {
    /// A TCP socket OpenOCD streams trace data to (`tpiu config internal :PORT`).
    Tcp(String),
    /// A captured raw-SWO file (handy for replay and validation).
    File(PathBuf),
}

impl Source {
    /// Open the source as an async byte reader.
    pub async fn open(&self) -> std::io::Result<Box<dyn AsyncRead + Unpin + Send>> {
        match self {
            Source::Tcp(addr) => Ok(Box::new(TcpStream::connect(addr).await?)),
            Source::File(path) => Ok(Box::new(tokio::fs::File::open(path).await?)),
        }
    }
}

/// Send OpenOCD (telnet console, default port 4444) the command sequence that
/// enables SWO/ITM capture and routes trace to an internal TCP port.
///
/// `cpu_hz` is the core clock and `swo_hz` the SWO clock from `[trace].swo_freq`.
/// This is best-effort: OpenOCD may already be configured, or use a different
/// command spelling across versions; failures here are non-fatal to tracing.
pub async fn openocd_enable(
    telnet_addr: &str,
    trace_port: u16,
    cpu_hz: u32,
    swo_hz: u32,
) -> std::io::Result<()> {
    let mut conn = TcpStream::connect(telnet_addr).await?;
    let commands = [
        format!("tpiu config internal :{trace_port} uart off {cpu_hz} {swo_hz}"),
        "itm ports on".to_string(),
    ];
    for cmd in commands {
        conn.write_all(cmd.as_bytes()).await?;
        conn.write_all(b"\r\n").await?;
    }
    conn.flush().await?;
    Ok(())
}
