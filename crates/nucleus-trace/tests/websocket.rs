//! End-to-end test of the trace daemon's WebSocket path: a real client connects,
//! bytes are fed to the server, and the client receives the decoded events as
//! JSON. This exercises the Phase 5 exit criterion ("pushes decoded events as
//! JSON over a WebSocket") without needing hardware.

use std::time::Duration;

use futures_util::StreamExt;
use nucleus_trace::{TraceServer, VarType, VariableMap};
use tokio_tungstenite::tungstenite::Message;

async fn recv_json(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> serde_json::Value {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timed out waiting for a WebSocket message")
        .expect("stream ended")
        .expect("websocket error");
    let text = msg.into_text().expect("expected a text frame");
    serde_json::from_str(&text).expect("expected valid JSON")
}

#[tokio::test]
async fn client_receives_decoded_events_as_json() {
    let mut vars = VariableMap::new();
    vars.insert(1, "temperature", VarType::F32);

    let server = TraceServer::new(vars);
    let (addr, _accept) = server.serve_ws("127.0.0.1:0").await.expect("bind ws");

    let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("connect");

    // The accept loop subscribes the client before completing the handshake, so
    // events emitted after connect_async returns are guaranteed delivered.
    server.ingest(b"\x01h\x01i\x01\n"); // port 0: "hi\n"
    let mut var_packet = vec![0x0Bu8]; // port 1, 4-byte payload
    var_packet.extend_from_slice(&21.5f32.to_le_bytes());
    server.ingest(&var_packet);

    let log = recv_json(&mut ws).await;
    assert_eq!(log["kind"], "log");
    assert_eq!(log["message"], "hi");

    let var = recv_json(&mut ws).await;
    assert_eq!(var["kind"], "variable");
    assert_eq!(var["name"], "temperature");
    assert_eq!(var["type"], "f32");
    assert_eq!(var["value"], 21.5);
}
