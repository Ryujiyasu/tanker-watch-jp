//! Coverage-days computation: how long Japan's crude stockpile + tracked
//! inbound tanker cargo lasts at the published consumption rate.
//!
//! Constants are deliberately conservative defaults reasonable for 2026:
//!   - daily consumption: 2.8M bbl/d (METI 2025 monthly avg)
//!   - stockpile baseline: 240 days (76d private mandate + 145d govt SPR
//!     + ~20d operating). Replace with live e-Stat / PAJ ingest later.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

pub const DAILY_CRUDE_CONSUMPTION_BBL: u64 = 2_800_000;
pub const STOCKPILE_DAYS_BASE: u64 = 240;

#[derive(Debug, Serialize)]
pub struct Runway {
    pub generated_at: DateTime<Utc>,
    pub daily_consumption_bbl: u64,
    pub stockpile_days_base: u64,
    pub stockpile_bbl: u64,
    pub pipeline_bbl: u64,
    pub pipeline_vessel_count: usize,
    pub pipeline_by_week: Vec<WeekBucket>,
    pub effective_runway_days: u64,
    pub inflow_per_day_bbl_30d: u64,
    pub deficit_per_day_bbl: i64, // + = drawing down, − = surplus
}

#[derive(Debug, Serialize)]
pub struct WeekBucket {
    pub week_start: DateTime<Utc>,
    pub bbl: u64,
    pub vessel_count: usize,
}

#[derive(Debug, Clone)]
pub struct PipelineEntry {
    pub cargo_bbl: u64,
    pub eta: Option<DateTime<Utc>>,
}

pub fn compute(entries: &[PipelineEntry]) -> Runway {
    let now = Utc::now();
    let stockpile_bbl = DAILY_CRUDE_CONSUMPTION_BBL * STOCKPILE_DAYS_BASE;
    let pipeline_bbl: u64 = entries.iter().map(|e| e.cargo_bbl).sum();

    let mut buckets: Vec<WeekBucket> = (0..8)
        .map(|i| WeekBucket {
            week_start: now + Duration::weeks(i),
            bbl: 0,
            vessel_count: 0,
        })
        .collect();

    for e in entries {
        if let Some(eta) = e.eta {
            let weeks = (eta - now).num_weeks();
            if weeks >= 0 && (weeks as usize) < buckets.len() {
                let b = &mut buckets[weeks as usize];
                b.bbl += e.cargo_bbl;
                b.vessel_count += 1;
            }
        }
    }

    let inflow_30d_bbl: u64 = entries
        .iter()
        .filter(|e| {
            e.eta
                .map(|eta| eta > now && eta - now < Duration::days(30))
                .unwrap_or(false)
        })
        .map(|e| e.cargo_bbl)
        .sum();
    let inflow_per_day = inflow_30d_bbl / 30;
    let deficit_per_day_bbl = DAILY_CRUDE_CONSUMPTION_BBL as i64 - inflow_per_day as i64;

    let effective_runway_days =
        (stockpile_bbl + pipeline_bbl) / DAILY_CRUDE_CONSUMPTION_BBL;

    Runway {
        generated_at: now,
        daily_consumption_bbl: DAILY_CRUDE_CONSUMPTION_BBL,
        stockpile_days_base: STOCKPILE_DAYS_BASE,
        stockpile_bbl,
        pipeline_bbl,
        pipeline_vessel_count: entries.len(),
        pipeline_by_week: buckets,
        effective_runway_days,
        inflow_per_day_bbl_30d: inflow_per_day,
        deficit_per_day_bbl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pipeline_yields_baseline_runway() {
        let r = compute(&[]);
        assert_eq!(r.effective_runway_days, STOCKPILE_DAYS_BASE);
        assert_eq!(r.pipeline_bbl, 0);
        assert_eq!(r.deficit_per_day_bbl, DAILY_CRUDE_CONSUMPTION_BBL as i64);
    }

    #[test]
    fn pipeline_extends_runway() {
        // 30 days of consumption worth in pipeline → +30 days runway
        let entries = vec![PipelineEntry {
            cargo_bbl: 30 * DAILY_CRUDE_CONSUMPTION_BBL,
            eta: Some(Utc::now() + Duration::days(7)),
        }];
        let r = compute(&entries);
        assert_eq!(r.effective_runway_days, STOCKPILE_DAYS_BASE + 30);
        assert_eq!(r.pipeline_vessel_count, 1);
    }
}
