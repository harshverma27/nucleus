//! The trace daemon: decode pipeline + WebSocket fan-out.
//!
//! A [`TraceServer`] owns the ITM decoder and the [`Translator`]. Bytes pushed
//! in via [`TraceServer::ingest`] are decoded, translated to [`TraceEvent`]s,
//! serialized to JSON, and broadcast to every connected WebSocket client. The
//! server is transport-agnostic: bytes can come from OpenOCD, a TCP trace port,
//! or a captured-file replay (see [`crate::source`]).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use nucleus_itm::Decoder;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::translate::{TraceEvent, Translator, VariableMap};

/// Capacity of the per-client broadcast queue. A slow client that falls this far
/// behind drops messages (logged as a lag) rather than stalling the pipeline.
const BROADCAST_CAPACITY: usize = 4096;

struct Pipeline {
    decoder: Decoder,
    translator: Translator,
}

/// The shared trace server. Clone the `Arc`, not the server.
pub struct TraceServer {
    tx: broadcast::Sender<String>,
    pipeline: Mutex<Pipeline>,
}

impl TraceServer {
    /// Build a server that decodes variables according to `vars`.
    pub fn new(vars: VariableMap) -> Arc<TraceServer> {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new(TraceServer {
            tx,
            pipeline: Mutex::new(Pipeline {
                decoder: Decoder::new(),
                translator: Translator::new(vars),
            }),
        })
    }

    /// Subscribe to the JSON event stream (one JSON object per message).
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Decode `bytes`, broadcast the resulting events, and return them (handy
    /// for tests and for callers that want to log locally).
    pub fn ingest(&self, bytes: &[u8]) -> Vec<TraceEvent> {
        let events = {
            let mut p = self.pipeline.lock().expect("pipeline mutex poisoned");
            let packets = p.decoder.decode(bytes);
            let mut events = Vec::new();
            for packet in &packets {
                events.extend(p.translator.translate(packet));
            }
            events
        };
        self.broadcast(&events);
        events
    }

    /// Emit any buffered partial log line. Call when the source ends.
    pub fn flush(&self) -> Vec<TraceEvent> {
        let events = {
            let mut p = self.pipeline.lock().expect("pipeline mutex poisoned");
            p.translator.flush()
        };
        self.broadcast(&events);
        events
    }

    fn broadcast(&self, events: &[TraceEvent]) {
        for event in events {
            if let Ok(json) = serde_json::to_string(event) {
                // `send` errors only when there are no subscribers; ignore.
                let _ = self.tx.send(json);
            }
        }
    }

    /// Bind a WebSocket listener on `addr` and accept clients in the background.
    /// Returns the bound address (useful when `addr` uses port 0).
    pub async fn serve_ws(
        self: &Arc<Self>,
        addr: &str,
    ) -> std::io::Result<(SocketAddr, JoinHandle<()>)> {
        let listener = TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        let server = Arc::clone(self);
        let handle = tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                let rx = server.subscribe();
                tokio::spawn(client_loop(stream, rx));
            }
        });
        Ok((local, handle))
    }

    /// Read SWO bytes from `reader` until EOF, feeding the pipeline.
    pub async fn run_source<R: AsyncRead + Unpin>(
        self: &Arc<Self>,
        mut reader: R,
    ) -> std::io::Result<()> {
        let mut buf = [0u8; 4096];
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            self.ingest(&buf[..n]);
        }
        self.flush();
        Ok(())
    }
}

/// Forward broadcast events to one WebSocket client until it disconnects.
async fn client_loop(stream: TcpStream, mut rx: broadcast::Receiver<String>) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(text) => {
                    if sink.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                // Slow consumer: skip dropped messages and keep going.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = source.next() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                Some(Ok(_)) => {} // clients are read-only; ignore their messages
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ingest_broadcasts_json_to_a_subscriber() {
        let mut vars = VariableMap::new();
        vars.insert(1, "temperature", crate::translate::VarType::F32);
        let server = TraceServer::new(vars);
        let mut rx = server.subscribe();

        // Port 0 "ok\n" then a port-1 f32.
        server.ingest(b"\x01o\x01k\x01\n");
        let mut stream = vec![0x0Bu8];
        stream.extend_from_slice(&2.0f32.to_le_bytes());
        server.ingest(&stream);

        let log: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(log["kind"], "log");
        assert_eq!(log["message"], "ok");

        let var: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(var["kind"], "variable");
        assert_eq!(var["name"], "temperature");
        assert_eq!(var["value"], 2.0);
    }
}
