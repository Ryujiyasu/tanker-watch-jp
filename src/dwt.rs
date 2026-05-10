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
}
