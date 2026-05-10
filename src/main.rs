mod db;

use anyhow::{Context, Result};
use chrono::Utc;
use db::DbWrite;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

const STREAM_URL: &str = "wss://stream.aisstream.io/v0/stream";
const DB_PATH: &str = "tanker.sqlite";

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

    let initial = db::load_tanker_mmsis(DB_PATH)?;
    info!(known_tankers = initial.len(), "loaded vessel catalog");
    let tanker_mmsis = Arc::new(RwLock::new(initial));

    let (db_tx, db_rx) = mpsc::channel::<DbWrite>(8192);
    let writer = db::spawn_writer(DB_PATH.into(), db_rx);

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
        if let Err(e) = handle(&txt, &tanker_mmsis, &db_tx).await {
            warn!(?e, "handle error");
        }
    }

    drop(db_tx);
    writer.await??;
    Ok(())
}

async fn handle(
    txt: &str,
    tanker_mmsis: &Arc<RwLock<HashSet<u64>>>,
    db_tx: &mpsc::Sender<DbWrite>,
) -> Result<()> {
    let v: Value = serde_json::from_str(txt)?;
    let mtype = v.get("MessageType").and_then(Value::as_str).unwrap_or("");
    let mmsi = match v.pointer("/MetaData/MMSI").and_then(Value::as_u64) {
        Some(m) => m,
        None => return Ok(()),
    };
    let ts = Utc::now().to_rfc3339();

    match mtype {
        "ShipStaticData" => {
            let s = v
                .pointer("/Message/ShipStaticData")
                .context("missing ShipStaticData body")?;
            let ship_type = s.get("Type").and_then(Value::as_u64);
            let imo = s.get("ImoNumber").and_then(Value::as_u64).filter(|&x| x > 0);
            let call_sign = s
                .get("CallSign")
                .and_then(Value::as_str)
                .map(|x| x.trim().to_owned())
                .filter(|x| !x.is_empty());
            let name = v
                .pointer("/MetaData/ShipName")
                .and_then(Value::as_str)
                .map(|x| x.trim().to_owned())
                .filter(|x| !x.is_empty());
            let dim_a = s.pointer("/Dimension/A").and_then(Value::as_u64);
            let dim_b = s.pointer("/Dimension/B").and_then(Value::as_u64);
            let dim_c = s.pointer("/Dimension/C").and_then(Value::as_u64);
            let dim_d = s.pointer("/Dimension/D").and_then(Value::as_u64);
            let draught = s
                .get("MaximumStaticDraught")
                .and_then(Value::as_f64)
                .filter(|&x| x > 0.0);
            let destination = s
                .get("Destination")
                .and_then(Value::as_str)
                .map(|x| x.trim().to_owned())
                .filter(|x| !x.is_empty());
            let eta = format_eta(s.get("Eta"));

            let is_tanker = matches!(ship_type, Some(80..=89));
            if is_tanker {
                tanker_mmsis.write().await.insert(mmsi);
            }
            db_tx
                .send(DbWrite::Vessel {
                    mmsi,
                    imo,
                    name: name.clone(),
                    call_sign,
                    ship_type,
                    dim_a,
                    dim_b,
                    dim_c,
                    dim_d,
                    ts: ts.clone(),
                })
                .await?;
            if is_tanker {
                db_tx
                    .send(DbWrite::Static {
                        mmsi,
                        draught,
                        destination: destination.clone(),
                        eta,
                        ts: ts.clone(),
                    })
                    .await?;
                info!(mmsi, ?name, ?ship_type, ?draught, ?destination, "tanker static");
            }
        }
        "PositionReport" => {
            if !tanker_mmsis.read().await.contains(&mmsi) {
                return Ok(());
            }
            let lat = v.pointer("/MetaData/latitude").and_then(Value::as_f64);
            let lon = v.pointer("/MetaData/longitude").and_then(Value::as_f64);
            let p = v.pointer("/Message/PositionReport");
            let sog = p.and_then(|x| x.get("Sog")).and_then(Value::as_f64);
            let cog = p.and_then(|x| x.get("Cog")).and_then(Value::as_f64);
            let heading = p
                .and_then(|x| x.get("TrueHeading"))
                .and_then(Value::as_f64);
            if let (Some(lat), Some(lon)) = (lat, lon) {
                db_tx
                    .send(DbWrite::Position {
                        mmsi,
                        lat,
                        lon,
                        sog,
                        cog,
                        heading,
                        ts,
                    })
                    .await?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn format_eta(eta: Option<&Value>) -> Option<String> {
    let eta = eta?;
    let m = eta.get("Month").and_then(Value::as_u64)?;
    let d = eta.get("Day").and_then(Value::as_u64)?;
    let h = eta.get("Hour").and_then(Value::as_u64)?;
    let mn = eta.get("Minute").and_then(Value::as_u64)?;
    if m == 0 || d == 0 {
        return None;
    }
    Some(format!("{m:02}-{d:02}T{h:02}:{mn:02}Z"))
}
