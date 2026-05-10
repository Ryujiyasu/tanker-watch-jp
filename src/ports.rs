//! Major Japan oil/crude terminals + AIS destination string matcher.
//!
//! AIS destination is captain-typed free text. Common patterns observed:
//!   "JP MIZ", ">JP YAT OFF", ">JP SBK 7-7 OFF//E", "JPCHB", "MIZUSHIMA"
//! UN/LOCODE is the canonical 5-char code (e.g. JPMZS for Mizushima),
//! but real-world entries often use ad-hoc 3-letter abbreviations.

#[derive(Debug, Clone)]
pub struct Port {
    pub code: &'static str,
    pub name_en: &'static str,
    pub name_ja: &'static str,
    pub aliases: &'static [&'static str],
}

pub const JP_PORTS: &[Port] = &[
    Port {
        code: "JPKII",
        name_en: "Kiire",
        name_ja: "喜入",
        aliases: &["KIIRE", "KII", "JP KII", "JPKII"],
    },
    Port {
        code: "JPMZS",
        name_en: "Mizushima",
        name_ja: "水島",
        aliases: &["MIZUSHIMA", "MIZ", "JP MIZ", "JPMZS"],
    },
    Port {
        code: "JPCHB",
        name_en: "Chiba",
        name_ja: "千葉",
        aliases: &["CHIBA", "CHB", "JP CHB", "JPCHB"],
    },
    Port {
        code: "JPKWS",
        name_en: "Kawasaki",
        name_ja: "川崎",
        aliases: &["KAWASAKI", "KWS", "JP KWS", "JPKWS"],
    },
    Port {
        code: "JPYOK",
        name_en: "Yokohama",
        name_ja: "横浜",
        aliases: &["YOKOHAMA", "YOK", "JP YOK", "JPYOK"],
    },
    Port {
        code: "JPYKK",
        name_en: "Yokkaichi",
        name_ja: "四日市",
        aliases: &["YOKKAICHI", "YKK", "JP YKK", "JPYKK"],
    },
    Port {
        code: "JPSDS",
        name_en: "Shimotsu",
        name_ja: "下津",
        aliases: &["SHIMOTSU", "SHM", "SDS", "JP SDS"],
    },
    Port {
        code: "JPSBS",
        name_en: "Shibushi",
        name_ja: "志布志",
        aliases: &["SHIBUSHI", "SBS", "JP SBS"],
    },
    Port {
        code: "JPNGO",
        name_en: "Nagoya",
        name_ja: "名古屋",
        aliases: &["NAGOYA", "NGO", "JP NGO", "JPNGO"],
    },
    Port {
        code: "JPSAK",
        name_en: "Sakai",
        name_ja: "堺",
        aliases: &["SAKAI", "JPSAK", "JP SAK"],
    },
    Port {
        code: "JPSAS",
        name_en: "Sasebo",
        name_ja: "佐世保",
        aliases: &["SASEBO", "SAS", "JPSAS"],
    },
    Port {
        code: "JPTOM",
        name_en: "Tomakomai",
        name_ja: "苫小牧",
        aliases: &["TOMAKOMAI", "TOM", "TMK", "JPTOM"],
    },
    Port {
        code: "JPMUT",
        name_en: "Mutsu Ogawara",
        name_ja: "むつ小川原",
        aliases: &["MUTSU", "OGAWARA", "MUT"],
    },
    Port {
        code: "JPYAT",
        name_en: "Yaizu",
        name_ja: "焼津",
        aliases: &["YAIZU", "YAT", "JP YAT"],
    },
];

pub fn match_port(dest: &str) -> Option<&'static Port> {
    let cleaned = dest
        .to_uppercase()
        .trim_start_matches('>')
        .trim_start_matches('<')
        .trim()
        .to_owned();
    for p in JP_PORTS {
        for a in p.aliases {
            if word_contains(&cleaned, a) {
                return Some(p);
            }
        }
    }
    None
}

fn word_contains(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(i, _)| {
        let before_ok = text[..i]
            .chars()
            .last()
            .map_or(true, |c| !c.is_ascii_alphanumeric());
        let after_idx = i + needle.len();
        let after_ok = text[after_idx..]
            .chars()
            .next()
            .map_or(true, |c| !c.is_ascii_alphanumeric());
        before_ok && after_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_observed_destinations() {
        assert_eq!(match_port("JP MIZ").unwrap().code, "JPMZS");
        assert_eq!(match_port(">JP YAT OFF").unwrap().code, "JPYAT");
        assert!(match_port(">JP SBK 7-7 OFF//E").is_none()); // SBK alias not in catalog
        assert_eq!(match_port("MIZUSHIMA").unwrap().code, "JPMZS");
        assert_eq!(match_port("JPCHB").unwrap().code, "JPCHB");
        assert_eq!(match_port("KAWASAKI").unwrap().code, "JPKWS");
        assert!(match_port("BUSAN").is_none());
        assert!(match_port("FOR ORDERS").is_none());
        assert!(match_port("SGSIN PEBGC").is_none());
    }

    #[test]
    fn word_boundary_avoids_false_positives() {
        // "MIZ" must not match "MIZUKAMI" or random substrings.
        assert!(match_port("KAMIZUKI").is_none());
        // But "MIZ" as standalone token does match.
        assert!(match_port("ETA MIZ 6/12").is_some());
    }
}
