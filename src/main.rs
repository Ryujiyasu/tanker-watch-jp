use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

const STREAM_URL: &str = "wss://stream.aisstream.io/v0/stream";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tanker_watch_jp=info".into()),
        )
        .init();

    let api_key = std::env::var("AISSTREAM_API_KEY")
        .context("set AISSTREAM_API_KEY (free key: https://aisstream.io)")?;

    let (mut ws, _) = connect_async(STREAM_URL).await?;
    info!("connected");

    let sub = json!({
        "APIKey": api_key,
        "BoundingBoxes": [
            [[24.0,  55.0], [27.0,  58.0]],   // Strait of Hormuz
            [[ 1.0,  98.0], [ 6.0, 105.0]],   // Malacca
            [[24.0, 122.0], [46.0, 146.0]],   // Japan approach
        ],
        "FilterMessageTypes": ["PositionReport", "ShipStaticData"]
    });
    ws.send(Message::Text(sub.to_string())).await?;

    while let Some(msg) = ws.next().await {
        let txt = match msg? {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => match std::str::from_utf8(&b) {
                Ok(s) => s.to_owned(),
                Err(_) => continue,
            },
            Message::Close(c) => {
                warn!(?c, "stream closed");
                break;
            }
            _ => continue,
        };
        if let Err(e) = handle(&txt) {
            warn!(?e, "handle error");
        }
    }
    Ok(())
}

fn handle(txt: &str) -> Result<()> {
    let v: Value = serde_json::from_str(txt)?;
    let mtype = v.get("MessageType").and_then(Value::as_str).unwrap_or("?");
    let mmsi = v
        .pointer("/MetaData/MMSI")
        .and_then(Value::as_u64);
    let name = v
        .pointer("/MetaData/ShipName")
        .and_then(Value::as_str)
        .map(str::trim);

    match mtype {
        "ShipStaticData" => {
            let ship_type = v.pointer("/Message/ShipStaticData/Type").and_then(Value::as_u64);
            let draught = v.pointer("/Message/ShipStaticData/MaximumStaticDraught").and_then(Value::as_f64);
            let dest = v.pointer("/Message/ShipStaticData/Destination").and_then(Value::as_str).map(str::trim);
            if matches!(ship_type, Some(80..=89)) {
                info!(?mmsi, ?name, ?ship_type, ?draught, ?dest, "TANKER static");
            }
        }
        "PositionReport" => {
            let lat = v.pointer("/MetaData/latitude").and_then(Value::as_f64);
            let lon = v.pointer("/MetaData/longitude").and_then(Value::as_f64);
            let sog = v.pointer("/Message/PositionReport/Sog").and_then(Value::as_f64);
            info!(?mmsi, ?name, ?lat, ?lon, ?sog, "pos");
        }
        _ => {}
    }
    Ok(())
}
