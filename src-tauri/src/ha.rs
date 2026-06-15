use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Convert a Home Assistant base URL into its WebSocket API endpoint.
pub fn ws_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let base = if let Some(rest) = trimmed.strip_prefix("https") {
        format!("wss{}", rest)
    } else if let Some(rest) = trimmed.strip_prefix("http") {
        format!("ws{}", rest)
    } else {
        trimmed.to_string()
    };
    format!("{}/api/websocket", base)
}

/// Connect to a single URL and complete the auth handshake.
pub async fn connect_authed(url: &str, token: &str) -> Result<Ws, String> {
    let endpoint = ws_url(url);
    let (mut ws, _) = connect_async(&endpoint).await.map_err(|e| e.to_string())?;
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(t.as_str()).map_err(|e| e.to_string())?;
                match v["type"].as_str() {
                    Some("auth_required") => {
                        let msg = json!({ "type": "auth", "access_token": token }).to_string();
                        ws.send(Message::Text(msg.into()))
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    Some("auth_ok") => return Ok(ws),
                    Some("auth_invalid") => return Err("Invalid access token.".to_string()),
                    _ => {}
                }
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("Connection closed.".to_string()),
        }
    }
}

/// Test each target URL independently and report per-URL results.
pub async fn test_auth(targets: Vec<(String, String)>, token: String) -> Value {
    if targets.is_empty() {
        return json!({ "ok": false, "error": "Enter your Home Assistant URL first." });
    }
    let mut results = Vec::new();
    for (label, url) in targets {
        let outcome =
            tokio::time::timeout(Duration::from_secs(9), connect_authed(&url, &token)).await;
        match outcome {
            Ok(Ok(mut ws)) => {
                let _ = ws.close(None).await;
                results.push(json!({ "label": label, "url": url, "ok": true }));
            }
            Ok(Err(e)) => results.push(json!({ "label": label, "url": url, "ok": false, "error": e })),
            Err(_) => results.push(
                json!({ "label": label, "url": url, "ok": false, "error": "Timed out." }),
            ),
        }
    }
    let ok = results
        .iter()
        .any(|r| r["ok"].as_bool().unwrap_or(false));
    json!({ "ok": ok, "results": results })
}

/// Fetch camera and binary_sensor entities (tries each URL in order).
pub async fn fetch_entities(targets: Vec<(String, String)>, token: String) -> Value {
    if targets.is_empty() {
        return json!({ "ok": false, "error": "Enter your Home Assistant URL first." });
    }
    let mut last_err = "Could not connect.".to_string();
    for (_label, url) in targets {
        match tokio::time::timeout(Duration::from_secs(9), connect_authed(&url, &token)).await {
            Ok(Ok(mut ws)) => {
                let req = json!({ "id": 1, "type": "get_states" }).to_string();
                if ws.send(Message::Text(req.into())).await.is_err() {
                    last_err = "Failed to send request.".to_string();
                    continue;
                }
                loop {
                    match tokio::time::timeout(Duration::from_secs(9), ws.next()).await {
                        Ok(Some(Ok(Message::Text(t)))) => {
                            let v: Value = match serde_json::from_str(t.as_str()) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            if v["type"] == "result" && v["id"] == 1 {
                                if !v["success"].as_bool().unwrap_or(false) {
                                    last_err = "Home Assistant rejected get_states.".to_string();
                                    break;
                                }
                                let states =
                                    v["result"].as_array().cloned().unwrap_or_default();
                                let _ = ws.close(None).await;
                                return parse_entities(states);
                            }
                        }
                        Ok(Some(Ok(_))) => {}
                        _ => {
                            last_err = "Connection closed.".to_string();
                            break;
                        }
                    }
                }
            }
            Ok(Err(e)) => last_err = e,
            Err(_) => last_err = "Timed out.".to_string(),
        }
    }
    json!({ "ok": false, "error": last_err })
}

fn parse_entities(states: Vec<Value>) -> Value {
    let mut cameras = Vec::new();
    let mut motion = Vec::new();
    for s in states {
        let entity_id = s["entity_id"].as_str().unwrap_or("").to_string();
        if entity_id.is_empty() {
            continue;
        }
        let domain = entity_id.split('.').next().unwrap_or("");
        let name = s["attributes"]["friendly_name"]
            .as_str()
            .unwrap_or(&entity_id)
            .to_string();
        if domain == "camera" {
            cameras.push(json!({ "entityId": entity_id, "name": name }));
        } else if domain == "binary_sensor" {
            let device_class = s["attributes"]["device_class"].as_str().map(|x| x.to_string());
            motion.push(json!({ "entityId": entity_id, "name": name, "deviceClass": device_class }));
        }
    }
    let by_name = |a: &Value, b: &Value| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    };
    cameras.sort_by(by_name);
    motion.sort_by(by_name);
    json!({ "ok": true, "cameras": cameras, "motion": motion })
}
