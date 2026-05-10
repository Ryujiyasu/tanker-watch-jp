use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub enum DwtSource {
    Lookup,
    Alpha,
}

#[derive(Debug, Clone, Copy)]
pub struct DwtEstimate {
    pub dwt: u64,
    pub source: DwtSource,
}

/// α regression for tankers: DWT ≈ 0.046 · LOA² · beam.
/// Calibrated against VLCC (330·60→320k), Suezmax (270·48→160k),
/// Aframax (245·42→110k), MR2 (180·32→48k). ±10–15% on type-known
/// crude/product carriers, larger error on chemical/asphalt/specialty.
pub fn alpha(loa_m: f64, beam_m: f64) -> u64 {
    (0.046 * loa_m * loa_m * beam_m).round() as u64
}

pub fn estimate(
    lookup: &HashMap<u64, u64>,
    imo: Option<u64>,
    loa: Option<u64>,
    beam: Option<u64>,
) -> Option<DwtEstimate> {
    if let Some(imo) = imo {
        if let Some(&dwt) = lookup.get(&imo) {
            return Some(DwtEstimate { dwt, source: DwtSource::Lookup });
        }
    }
    match (loa, beam) {
        (Some(l), Some(b)) if l > 0 && b > 0 => Some(DwtEstimate {
            dwt: alpha(l as f64, b as f64),
            source: DwtSource::Alpha,
        }),
        _ => None,
    }
}

/// Cargo estimate from AIS draught and DWT, assuming crude-equivalent.
#[derive(Debug, Clone, Copy)]
pub struct CargoEstimate {
    pub t_design_m: f64,
    pub t_ballast_m: f64,
    pub load_ratio: f64,
    pub cargo_tonnes: u64,
    pub cargo_bbl_crude: u64,
}

/// Convert AIS-reported draught into crude-equivalent cargo.
///
/// Empirical model:
///   T_design   ≈ LOA / 15.5      (tanker L/T ratio, ±5% across classes)
///   T_ballast  ≈ 0.4 · T_design  (typical ballast draft)
///   load_ratio = clamp01((draught − T_ballast) / (T_design − T_ballast))
///   cargo_t    = DWT · load_ratio
///   cargo_bbl  = cargo_t · 7.33  (crude ~0.86 t/m³, API ~32)
///
/// Caveats: ship_type 80-89 lumps crude / product / chemical / LNG / LPG.
/// Density ranges 0.55 (LPG) to 1.0+ (bitumen). bbl figure is
/// **crude-equivalent**; refine via dwt_lookup carrying product class.
pub fn estimate_cargo(loa_m: f64, draught_m: f64, dwt_t: u64) -> Option<CargoEstimate> {
    if loa_m <= 0.0 || draught_m <= 0.0 || dwt_t == 0 {
        return None;
    }
    let t_design = loa_m / 15.5;
    let t_ballast = 0.4 * t_design;
    if t_design <= t_ballast {
        return None;
    }
    let raw = (draught_m - t_ballast) / (t_design - t_ballast);
    let load_ratio = raw.clamp(0.0, 1.0);
    let cargo_t = (dwt_t as f64) * load_ratio;
    Some(CargoEstimate {
        t_design_m: t_design,
        t_ballast_m: t_ballast,
        load_ratio,
        cargo_tonnes: cargo_t.round() as u64,
        cargo_bbl_crude: (cargo_t * 7.33).round() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_calibration() {
        // Known tanker class anchors, ±15% tolerance.
        let cases = [
            (330.0, 60.0, 300_000.0),
            (270.0, 48.0, 160_000.0),
            (245.0, 42.0, 110_000.0),
            (180.0, 32.0, 50_000.0),
        ];
        for (l, b, expected) in cases {
            let got = alpha(l, b) as f64;
            let err = (got - expected).abs() / expected;
            assert!(
                err < 0.15,
                "loa={l} beam={b} expected≈{expected} got={got} err={err}"
            );
        }
    }

    #[test]
    fn cargo_eneos_ocean_half_loaded() {
        // ENEOS OCEAN captured live: LOA 340, AIS draught 15.3m,
        // α DWT 319,000. Expected ~50% load → ~1.15M bbl crude.
        let dwt = alpha(340.0, 60.0);
        let est = estimate_cargo(340.0, 15.3, dwt).unwrap();
        assert!((est.load_ratio - 0.50).abs() < 0.05, "load_ratio={}", est.load_ratio);
        assert!(
            (est.cargo_bbl_crude as i64 - 1_160_000).abs() < 100_000,
            "cargo_bbl_crude={}",
            est.cargo_bbl_crude
        );
    }

    #[test]
    fn cargo_clamps_below_ballast_to_zero() {
        let dwt = alpha(330.0, 60.0);
        // draught well below ballast (8.8m) → 0 cargo.
        let est = estimate_cargo(330.0, 5.0, dwt).unwrap();
        assert_eq!(est.load_ratio, 0.0);
        assert_eq!(est.cargo_bbl_crude, 0);
    }

    #[test]
    fn cargo_clamps_above_design_to_full() {
        let dwt = alpha(330.0, 60.0);
        // draught above T_design = 21.3m → fully loaded.
        let est = estimate_cargo(330.0, 25.0, dwt).unwrap();
        assert_eq!(est.load_ratio, 1.0);
    }
}
