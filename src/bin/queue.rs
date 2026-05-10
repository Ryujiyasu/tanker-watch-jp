//! Print the current Japan-port arrival queue from `tanker.sqlite`.
//!
//! Joins the latest `static_history` row per tanker MMSI with the vessel
//! catalog, classifies destination via `ports::match_port`, estimates DWT
//! and crude-equivalent cargo via `dwt`, and prints a per-port queue
//! sorted by ETA.

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Utc};
use std::collections::HashMap;
use tanker_watch_jp::{
    db, dwt,
    ports::{self, Port, JP_PORTS},
};

const DB_PATH: &str = "tanker.sqlite";

#[derive(Debug)]
struct Arrival {
    name: String,
    loa: Option<u64>,
    beam: Option<u64>,
    draught: Option<f64>,
    destination: String,
    eta: Option<DateTime<Utc>>,
    dwt_t: Option<u64>,
    cargo_bbl: Option<u64>,
}

fn main() -> Result<()> {
    let conn = db::open_with_schema(DB_PATH)?;

    let mut stmt = conn.prepare(
        r#"SELECT v.name, v.dim_a + v.dim_b AS loa, v.dim_c + v.dim_d AS beam,
                  s.draught, s.destination, s.eta
           FROM vessels v
           JOIN (
               SELECT mmsi, draught, destination, eta, ts FROM static_history sh
               WHERE rowid IN (SELECT MAX(rowid) FROM static_history GROUP BY mmsi)
           ) s ON s.mmsi = v.mmsi
           WHERE v.ship_type BETWEEN 80 AND 89
             AND s.destination IS NOT NULL"#,
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(1)?.map(|x| x as u64),
            row.get::<_, Option<i64>>(2)?.map(|x| x as u64),
            row.get::<_, Option<f64>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;

    let dwt_lookup = HashMap::new(); // empty for now; populated by β'

    let mut by_port: HashMap<&'static str, Vec<Arrival>> = HashMap::new();
    let mut total = 0;
    let mut classified = 0;

    for r in rows {
        let (name, loa, beam, draught, destination, eta_str) = r?;
        total += 1;
        let port = match ports::match_port(&destination) {
            Some(p) => p,
            None => continue,
        };
        classified += 1;

        let est = dwt::estimate(&dwt_lookup, None, loa, beam);
        let cargo = match (loa, draught, est) {
            (Some(l), Some(d), Some(e)) => dwt::estimate_cargo(l as f64, d, e.dwt),
            _ => None,
        };

        by_port.entry(port.code).or_default().push(Arrival {
            name,
            loa,
            beam,
            draught,
            destination,
            eta: eta_str.as_deref().and_then(parse_eta),
            dwt_t: est.map(|e| e.dwt),
            cargo_bbl: cargo.map(|c| c.cargo_bbl_crude),
        });
    }

    print_queues(&by_port);

    println!();
    println!("--- summary ---");
    println!(
        "tanker static rows : {}\nclassified to port : {}\nunmatched          : {}",
        total,
        classified,
        total - classified
    );

    Ok(())
}

fn parse_eta(eta: &str) -> Option<DateTime<Utc>> {
    // "MM-DDTHH:MMZ" → "YYYY-MM-DDTHH:MM:00Z"
    let now = Utc::now();
    let trimmed = eta.trim_end_matches('Z');
    let s = format!("{}-{trimmed}:00Z", now.year());
    let parsed: DateTime<Utc> = s.parse().ok()?;
    if parsed < now - Duration::days(30) {
        // Year rolled over (December ETA seen in January, etc.)
        let next = format!("{}-{trimmed}:00Z", now.year() + 1);
        return next.parse().ok();
    }
    Some(parsed)
}

fn print_queues(by_port: &HashMap<&'static str, Vec<Arrival>>) {
    let mut ordered: Vec<&Port> = JP_PORTS
        .iter()
        .filter(|p| by_port.contains_key(p.code))
        .collect();
    ordered.sort_by_key(|p| p.code);

    for port in ordered {
        let mut arrivals: Vec<_> = by_port[port.code].iter().collect();
        arrivals.sort_by_key(|a| a.eta.unwrap_or(DateTime::<Utc>::MAX_UTC));

        println!(
            "\n=== {} {} ({}) — {} 隻 ===",
            port.name_ja,
            port.name_en,
            port.code,
            arrivals.len(),
        );
        println!(
            "{:<20} {:>6} {:>6} {:>8} {:>10} {:>14}  {:<22} ETA",
            "name", "LOA", "beam", "draught", "DWT(t)", "cargo(bbl)", "destination",
        );
        for a in arrivals {
            let eta_str = a
                .eta
                .map(|e| e.format("%m-%d %H:%MZ").to_string())
                .unwrap_or_else(|| "—".into());
            println!(
                "{:<20} {:>6} {:>6} {:>8} {:>10} {:>14}  {:<22} {}",
                truncate(&a.name, 20),
                fmt_u64(a.loa),
                fmt_u64(a.beam),
                a.draught
                    .map(|d| format!("{d:.1}m"))
                    .unwrap_or_else(|| "—".into()),
                fmt_u64(a.dwt_t),
                a.cargo_bbl
                    .map(|b| group_thousands(b))
                    .unwrap_or_else(|| "—".into()),
                truncate(&a.destination, 22),
                eta_str,
            );
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

fn fmt_u64(v: Option<u64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "—".into())
}

fn group_thousands(v: u64) -> String {
    let s = v.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}
