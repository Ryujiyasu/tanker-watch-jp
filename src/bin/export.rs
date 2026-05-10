//! Snapshot the SQLite ingest into static JSON files for the web frontend.
//!
//! Outputs to ./web/data/:
//!   positions.json  — GeoJSON FeatureCollection of recent tanker positions
//!   arrivals.json   — per-port arrival queue
//!   meta.json       — generated_at timestamp + counts

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Utc};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use tanker_watch_jp::{
    db, dwt,
    ports::{self, JP_PORTS},
};

const DB_PATH: &str = "tanker.sqlite";
const OUT_DIR: &str = "web/data";

#[derive(Serialize)]
struct FeatureCollection {
    #[serde(rename = "type")]
    typ: &'static str,
    features: Vec<Feature>,
}

#[derive(Serialize)]
struct Feature {
    #[serde(rename = "type")]
    typ: &'static str,
    geometry: Geometry,
    properties: Props,
}

#[derive(Serialize)]
struct Geometry {
    #[serde(rename = "type")]
    typ: &'static str,
    coordinates: [f64; 2],
}

#[derive(Serialize)]
struct Props {
    mmsi: u64,
    imo: Option<u64>,
    name: String,
    loa: Option<u64>,
    sog: Option<f64>,
    heading: Option<f64>,
    draught: Option<f64>,
    destination: Option<String>,
    eta: Option<String>,
    dwt_t: Option<u64>,
    cargo_bbl: Option<u64>,
    load_pct: Option<u64>,
    port_code: Option<&'static str>,
    port_name_ja: Option<&'static str>,
}

#[derive(Serialize)]
struct PortQueue {
    code: &'static str,
    name_en: &'static str,
    name_ja: &'static str,
    arrivals: Vec<Arrival>,
}

#[derive(Serialize)]
struct Arrival {
    mmsi: u64,
    name: String,
    loa: Option<u64>,
    draught: Option<f64>,
    dwt_t: Option<u64>,
    cargo_bbl: Option<u64>,
    load_pct: Option<u64>,
    destination: String,
    eta: Option<DateTime<Utc>>,
}

fn main() -> Result<()> {
    let conn = db::open_with_schema(DB_PATH)?;
    fs::create_dir_all(OUT_DIR)?;
    let dwt_lookup: HashMap<u64, u64> = HashMap::new();

    let positions = build_positions(&conn, &dwt_lookup)?;
    let arrivals = build_arrivals(&conn, &dwt_lookup)?;

    fs::write(
        format!("{OUT_DIR}/positions.json"),
        serde_json::to_string(&positions)?,
    )?;
    fs::write(
        format!("{OUT_DIR}/arrivals.json"),
        serde_json::to_string(&arrivals)?,
    )?;
    fs::write(
        format!("{OUT_DIR}/meta.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "generated_at": Utc::now().to_rfc3339(),
            "vessel_count": positions.features.len(),
            "port_count": arrivals.iter().filter(|p| !p.arrivals.is_empty()).count(),
            "arrival_count": arrivals.iter().map(|p| p.arrivals.len()).sum::<usize>(),
        }))?,
    )?;

    println!(
        "exported: {} vessels, {} arrivals across {} ports → {}/",
        positions.features.len(),
        arrivals.iter().map(|p| p.arrivals.len()).sum::<usize>(),
        arrivals.iter().filter(|p| !p.arrivals.is_empty()).count(),
        OUT_DIR,
    );
    Ok(())
}

fn build_positions(
    conn: &Connection,
    dwt_lookup: &HashMap<u64, u64>,
) -> Result<FeatureCollection> {
    let cutoff = (Utc::now() - Duration::hours(24)).to_rfc3339();
    let mut stmt = conn.prepare(
        r#"SELECT v.mmsi, v.imo, v.name, v.dim_a+v.dim_b AS loa, v.dim_c+v.dim_d AS beam,
                  p.lat, p.lon, p.sog, p.heading,
                  s.draught, s.destination, s.eta
           FROM vessels v
           JOIN (SELECT mmsi, lat, lon, sog, heading, ts FROM positions
                 WHERE rowid IN (SELECT MAX(rowid) FROM positions GROUP BY mmsi)) p
             ON p.mmsi = v.mmsi
           LEFT JOIN (SELECT mmsi, draught, destination, eta FROM static_history
                      WHERE rowid IN (SELECT MAX(rowid) FROM static_history GROUP BY mmsi)) s
             ON s.mmsi = v.mmsi
           WHERE v.ship_type BETWEEN 80 AND 89 AND p.ts > ?1"#,
    )?;

    let mut features = Vec::new();
    let rows = stmt.query_map([&cutoff], |row| {
        Ok((
            row.get::<_, i64>(0)? as u64,
            row.get::<_, Option<i64>>(1)?.map(|x| x as u64),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.map(|x| x as u64),
            row.get::<_, Option<i64>>(4)?.map(|x| x as u64),
            row.get::<_, f64>(5)?,
            row.get::<_, f64>(6)?,
            row.get::<_, Option<f64>>(7)?,
            row.get::<_, Option<f64>>(8)?,
            row.get::<_, Option<f64>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
        ))
    })?;

    for r in rows {
        let (mmsi, imo, name, loa, beam, lat, lon, sog, heading, draught, destination, eta) = r?;
        let est = dwt::estimate(dwt_lookup, imo, loa, beam);
        let cargo = match (loa, draught, est) {
            (Some(l), Some(d), Some(e)) => dwt::estimate_cargo(l as f64, d, e.dwt),
            _ => None,
        };
        let port = destination.as_deref().and_then(ports::match_port);

        features.push(Feature {
            typ: "Feature",
            geometry: Geometry {
                typ: "Point",
                coordinates: [lon, lat],
            },
            properties: Props {
                mmsi,
                imo,
                name,
                loa,
                sog,
                heading,
                draught,
                destination,
                eta,
                dwt_t: est.map(|e| e.dwt),
                cargo_bbl: cargo.map(|c| c.cargo_bbl_crude),
                load_pct: cargo.map(|c| (c.load_ratio * 100.0).round() as u64),
                port_code: port.map(|p| p.code),
                port_name_ja: port.map(|p| p.name_ja),
            },
        });
    }

    Ok(FeatureCollection {
        typ: "FeatureCollection",
        features,
    })
}

fn build_arrivals(
    conn: &Connection,
    dwt_lookup: &HashMap<u64, u64>,
) -> Result<Vec<PortQueue>> {
    let mut stmt = conn.prepare(
        r#"SELECT v.mmsi, v.imo, v.name, v.dim_a+v.dim_b AS loa, v.dim_c+v.dim_d AS beam,
                  s.draught, s.destination, s.eta
           FROM vessels v
           JOIN (SELECT mmsi, draught, destination, eta, ts FROM static_history
                 WHERE rowid IN (SELECT MAX(rowid) FROM static_history GROUP BY mmsi)) s
             ON s.mmsi = v.mmsi
           WHERE v.ship_type BETWEEN 80 AND 89 AND s.destination IS NOT NULL"#,
    )?;

    let mut bucket: HashMap<&'static str, Vec<Arrival>> = HashMap::new();

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)? as u64,
            row.get::<_, Option<i64>>(1)?.map(|x| x as u64),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.map(|x| x as u64),
            row.get::<_, Option<i64>>(4)?.map(|x| x as u64),
            row.get::<_, Option<f64>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;

    for r in rows {
        let (mmsi, imo, name, loa, beam, draught, destination, eta_str) = r?;
        let port = match ports::match_port(&destination) {
            Some(p) => p,
            None => continue,
        };
        let est = dwt::estimate(dwt_lookup, imo, loa, beam);
        let cargo = match (loa, draught, est) {
            (Some(l), Some(d), Some(e)) => dwt::estimate_cargo(l as f64, d, e.dwt),
            _ => None,
        };
        bucket.entry(port.code).or_default().push(Arrival {
            mmsi,
            name,
            loa,
            draught,
            dwt_t: est.map(|e| e.dwt),
            cargo_bbl: cargo.map(|c| c.cargo_bbl_crude),
            load_pct: cargo.map(|c| (c.load_ratio * 100.0).round() as u64),
            destination,
            eta: eta_str.as_deref().and_then(parse_eta),
        });
    }

    let mut queues: Vec<PortQueue> = JP_PORTS
        .iter()
        .map(|p| {
            let mut arr = bucket.remove(p.code).unwrap_or_default();
            arr.sort_by_key(|a| a.eta.unwrap_or(DateTime::<Utc>::MAX_UTC));
            PortQueue {
                code: p.code,
                name_en: p.name_en,
                name_ja: p.name_ja,
                arrivals: arr,
            }
        })
        .collect();
    queues.retain(|q| !q.arrivals.is_empty());
    Ok(queues)
}

fn parse_eta(eta: &str) -> Option<DateTime<Utc>> {
    let now = Utc::now();
    let trimmed = eta.trim_end_matches('Z');
    let s = format!("{}-{trimmed}:00Z", now.year());
    let parsed: DateTime<Utc> = s.parse().ok()?;
    if parsed < now - Duration::days(30) {
        let next = format!("{}-{trimmed}:00Z", now.year() + 1);
        return next.parse().ok();
    }
    Some(parsed)
}
