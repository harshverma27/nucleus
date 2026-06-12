//! The Nucleus trace backend.
//!
//! Wires the [`nucleus_itm`] decoder to a WebSocket fan-out so editor/browser
//! dashboards can subscribe to live, structured trace events:
//!
//! ```text
//! OpenOCD (SWO) ──▶ source ──▶ Decoder ──▶ Translator ──▶ JSON ──▶ WebSocket :7878
//! ```
//!
//! [`translate`] assigns meaning (port 0 = logs, ports 1–7 = typed variables),
//! [`server`] owns the pipeline and client fan-out, and [`source`] supplies the
//! bytes (OpenOCD TCP trace port or a captured-file replay). [`run_blocking`] is
//! the synchronous entry point the CLI calls.

pub mod server;
pub mod source;
pub mod translate;

pub use server::TraceServer;
pub use source::Source;
pub use translate::{TraceEvent, Translator, VarType, VariableMap};

/// The default WebSocket port (per the README/architecture).
pub const DEFAULT_WS_PORT: u16 = 7878;

/// Options for [`run_blocking`].
pub struct TraceOptions {
    /// Address to bind the WebSocket server on, e.g. `127.0.0.1:7878`.
    pub ws_addr: String,
    /// Where SWO bytes come from.
    pub source: Source,
    /// Optional OpenOCD telnet setup: `(telnet_addr, trace_port, cpu_hz, swo_hz)`.
    pub openocd: Option<(String, u16, u32, u32)>,
    /// Port → variable map from `[[trace.variables]]`.
    pub variables: VariableMap,
}

/// Run the trace daemon to completion (until the source ends), blocking on a
/// fresh Tokio runtime so the caller stays synchronous.
pub fn run_blocking(opts: TraceOptions) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let server = TraceServer::new(opts.variables);
        let (addr, _accept) = server.serve_ws(&opts.ws_addr).await?;
        eprintln!("nucleus trace: streaming events on ws://{addr}");

        if let Some((telnet, port, cpu, swo)) = &opts.openocd {
            if let Err(err) = source::openocd_enable(telnet, *port, *cpu, *swo).await {
                eprintln!("nucleus trace: warning: OpenOCD setup failed ({err}); continuing");
            }
        }

        let reader = opts.source.open().await?;
        eprintln!("nucleus trace: reading SWO from source…");
        server.run_source(reader).await?;
        eprintln!("nucleus trace: source ended");
        Ok(())
    })
}
