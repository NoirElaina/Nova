use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use nova_client::uds::{UnixListener, prepare_private_socket_directory};
use nova_client::{Client, ClientOptions};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn uds_handshake_rejects_incompatible_protocol_version() {
    let dir = PathBuf::from(format!(
        "{}/nova-ver-{}",
        std::env::temp_dir().display(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    prepare_private_socket_directory(&dir)
        .await
        .expect("prepare private socket dir");
    let socket_path = dir.join("daemon.sock");

    let mut listener = UnixListener::bind(&socket_path)
        .await
        .expect("bind test UDS listener");

    let server = tokio::spawn(async move {
        let stream = listener.accept().await.expect("accept client");
        stream.ensure_trusted_peer().expect("trusted local peer");
        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .expect("ws accept");
        let (mut sink, mut rx) = ws.split();
        while let Some(Ok(msg)) = rx.next().await {
            let Message::Text(text) = msg else { continue };
            let req: Value = serde_json::from_str(&text).expect("valid json");
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            if req.get("method").and_then(Value::as_str) == Some("initialize") {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "serverInfo": {
                            "name": "nova",
                            "title": "Stale Daemon",
                            "version": "0.0.1-stale"
                        },
                        "protocolVersion": "999",
                        "capabilities": {
                            "browser": true,
                            "hooks": true,
                            "escalation": true
                        }
                    }
                });
                let _ = sink.send(Message::Text(resp.to_string().into())).await;
                break;
            }
        }
    });

    let err = match Client::connect_uds(&socket_path, ClientOptions::default()).await {
        Ok(_) => panic!("incompatible daemon version must be rejected"),
        Err(err) => err,
    };
    assert!(
        err.contains("协议或版本不匹配"),
        "unexpected error message: {err}"
    );

    let _ = server.await;
    let _ = std::fs::remove_dir_all(&dir);
}
