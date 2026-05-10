use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::collections::HashSet;
use tokio::sync::mpsc;

pub enum DbWrite {
    Vessel {
        mmsi: u64,
        imo: Option<u64>,
        name: Option<String>,
        call_sign: Option<String>,
        ship_type: Option<u64>,
        dim_a: Option<u64>,
        dim_b: Option<u64>,
        dim_c: Option<u64>,
        dim_d: Option<u64>,
        ts: String,
    },
    Position {
        mmsi: u64,
        lat: f64,
        lon: f64,
        sog: Option<f64>,
        cog: Option<f64>,
        heading: Option<f64>,
        ts: String,
    },
    Static {
        mmsi: u64,
        draught: Option<f64>,
        destination: Option<String>,
        eta: Option<String>,
        ts: String,
    },
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS vessels (
    mmsi       INTEGER PRIMARY KEY,
    imo        INTEGER,
    name       TEXT,
    call_sign  TEXT,
    ship_type  INTEGER,
    dim_a      INTEGER,
    dim_b      INTEGER,
    dim_c      INTEGER,
    dim_d      INTEGER,
    first_seen TEXT NOT NULL,
    last_seen  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_vessels_ship_type ON vessels(ship_type);

CREATE TABLE IF NOT EXISTS positions (
    mmsi    INTEGER NOT NULL,
    ts      TEXT    NOT NULL,
    lat     REAL    NOT NULL,
    lon     REAL    NOT NULL,
    sog     REAL,
    cog     REAL,
    heading REAL,
    PRIMARY KEY (mmsi, ts)
);
CREATE INDEX IF NOT EXISTS idx_positions_ts ON positions(ts);

CREATE TABLE IF NOT EXISTS static_history (
    mmsi        INTEGER NOT NULL,
    ts          TEXT    NOT NULL,
    draught     REAL,
    destination TEXT,
    eta         TEXT,
    PRIMARY KEY (mmsi, ts)
);
"#;

fn open_with_schema(path: &str) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("open {path}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA).context("apply schema")?;
    Ok(conn)
}

pub fn load_tanker_mmsis(path: &str) -> Result<HashSet<u64>> {
    let conn = open_with_schema(path)?;
    let mut stmt =
        conn.prepare("SELECT mmsi FROM vessels WHERE ship_type BETWEEN 80 AND 89")?;
    let mut set = HashSet::new();
    let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    for r in rows {
        set.insert(r? as u64);
    }
    Ok(set)
}

pub fn spawn_writer(
    path: String,
    mut rx: mpsc::Receiver<DbWrite>,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = open_with_schema(&path)?;
        while let Some(msg) = rx.blocking_recv() {
            if let Err(e) = apply(&conn, msg) {
                tracing::warn!(?e, "db write failed");
            }
        }
        Ok(())
    })
}

fn apply(conn: &Connection, msg: DbWrite) -> Result<()> {
    match msg {
        DbWrite::Vessel {
            mmsi, imo, name, call_sign, ship_type,
            dim_a, dim_b, dim_c, dim_d, ts,
        } => {
            conn.execute(
                r#"INSERT INTO vessels
                     (mmsi, imo, name, call_sign, ship_type, dim_a, dim_b, dim_c, dim_d, first_seen, last_seen)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                   ON CONFLICT(mmsi) DO UPDATE SET
                     imo       = COALESCE(excluded.imo,       vessels.imo),
                     name      = COALESCE(excluded.name,      vessels.name),
                     call_sign = COALESCE(excluded.call_sign, vessels.call_sign),
                     ship_type = COALESCE(excluded.ship_type, vessels.ship_type),
                     dim_a     = COALESCE(excluded.dim_a,     vessels.dim_a),
                     dim_b     = COALESCE(excluded.dim_b,     vessels.dim_b),
                     dim_c     = COALESCE(excluded.dim_c,     vessels.dim_c),
                     dim_d     = COALESCE(excluded.dim_d,     vessels.dim_d),
                     last_seen = excluded.last_seen"#,
                params![
                    mmsi as i64,
                    imo.map(|x| x as i64),
                    name,
                    call_sign,
                    ship_type.map(|x| x as i64),
                    dim_a.map(|x| x as i64),
                    dim_b.map(|x| x as i64),
                    dim_c.map(|x| x as i64),
                    dim_d.map(|x| x as i64),
                    ts,
                ],
            )?;
        }
        DbWrite::Position { mmsi, lat, lon, sog, cog, heading, ts } => {
            conn.execute(
                "INSERT OR IGNORE INTO positions
                   (mmsi, ts, lat, lon, sog, cog, heading)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![mmsi as i64, ts, lat, lon, sog, cog, heading],
            )?;
        }
        DbWrite::Static { mmsi, draught, destination, eta, ts } => {
            conn.execute(
                "INSERT OR IGNORE INTO static_history
                   (mmsi, ts, draught, destination, eta)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![mmsi as i64, ts, draught, destination, eta],
            )?;
        }
    }
    Ok(())
}
