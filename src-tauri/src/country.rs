//! Offline IPv4 country lookup and AMX Mod X language-to-flag mapping.

use std::net::Ipv4Addr;
use std::sync::OnceLock;

// sapics/ip-location-db, server-country IPv4 numeric release (PDDL).
// Numeric start/end ranges are sorted, so one binary search answers a row.
const RANGES: &str = include_str!("../data/server-country-ipv4-num.csv");
static PARSED: OnceLock<Vec<(u32, u32, &'static str)>> = OnceLock::new();

pub fn from_ip(ip: Ipv4Addr) -> Option<&'static str> {
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
    {
        return None;
    }
    let ranges = PARSED.get_or_init(|| {
        RANGES
            .lines()
            .filter_map(|line| {
                let mut cells = line.split(',');
                Some((
                    cells.next()?.parse().ok()?,
                    cells.next()?.parse().ok()?,
                    cells.next()?,
                ))
            })
            .collect()
    });
    let value = u32::from(ip);
    let index = ranges.partition_point(|(start, _, _)| *start <= value);
    let (_, end, code) = ranges.get(index.checked_sub(1)?)?;
    if value > *end || !valid_code(code) {
        return None;
    }
    // The dataset retains the old FX code for metropolitan France; the flag
    // package uses today's FR code.
    Some(if *code == "FX" { "FR" } else { *code })
}

/// AMXX uses ISO 639 language codes; a flag needs an ISO 3166 country.
/// Some languages span countries, so these are representative flags.
pub fn from_amx_language(value: &str) -> Option<&'static str> {
    let language = value.trim().trim_matches('"').to_ascii_lowercase();
    Some(match language.as_str() {
        "en" => "GB",
        "de" => "DE",
        "fr" => "FR",
        "es" => "ES",
        "it" => "IT",
        "pt" => "PT",
        "bp" | "br" => "BR",
        "ru" => "RU",
        "uk" | "ua" => "UA",
        "pl" => "PL",
        "cs" | "cz" => "CZ",
        "sk" => "SK",
        "hu" => "HU",
        "ro" => "RO",
        "bg" => "BG",
        "sr" => "RS",
        "hr" => "HR",
        "bs" => "BA",
        "sl" => "SI",
        "nl" => "NL",
        "sv" => "SE",
        "da" => "DK",
        "no" => "NO",
        "fi" => "FI",
        "el" | "gr" => "GR",
        "tr" => "TR",
        "he" => "IL",
        "ar" => "SA",
        "fa" => "IR",
        "hi" => "IN",
        "id" => "ID",
        "vi" => "VN",
        "ja" => "JP",
        "ko" => "KR",
        "zh" | "cn" => "CN",
        "tw" => "TW",
        "th" => "TH",
        "ms" => "MY",
        "et" => "EE",
        "lt" => "LT",
        "lv" => "LV",
        "al" | "sq" => "AL",
        "az" => "AZ",
        "be" => "BY",
        "mk" => "MK",
        _ => return None,
    })
}

pub fn from_rules(rules: &[(String, String)]) -> Option<&'static str> {
    rules
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("amx_language"))
        .and_then(|(_, value)| from_amx_language(value))
}

pub fn resolve(ip: Ipv4Addr, rules: Option<&[(String, String)]>) -> Option<&'static str> {
    rules.and_then(from_rules).or_else(|| from_ip(ip))
}

fn valid_code(code: &str) -> bool {
    code.len() == 2 && code != "ZZ" && code.bytes().all(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_wins_only_when_recognized() {
        assert_eq!(
            from_rules(&[("AMX_LANGUAGE".into(), " bp ".into())]),
            Some("BR")
        );
        assert_eq!(from_rules(&[("amx_language".into(), "???".into())]), None);
        assert_eq!(from_amx_language("en"), Some("GB"));
        let ip = "8.8.8.8".parse().unwrap();
        assert_eq!(
            resolve(ip, Some(&[("amx_language".into(), "pl".into())])),
            Some("PL")
        );
        assert_eq!(
            resolve(ip, Some(&[("amx_language".into(), "???".into())])),
            Some("US")
        );
    }

    #[test]
    fn covers_every_language_in_amxmodx_languages_txt() {
        // alliedmodders/amxmodx plugins/lang/languages.txt (master).
        for code in [
            "en", "de", "sr", "tr", "fr", "sv", "da", "pl", "nl", "es", "bp", "cz", "fi", "bg",
            "ro", "hu", "lt", "sk", "mk", "hr", "bs", "ru", "cn", "al", "pt",
        ] {
            assert!(from_amx_language(code).is_some(), "missing {code}");
        }
    }

    #[test]
    fn lookup_known_and_unknown_addresses() {
        assert_eq!(from_ip("8.8.8.8".parse().unwrap()), Some("US"));
        assert_eq!(from_ip(Ipv4Addr::from(2_954_335_232)), Some("FR"));
        assert_eq!(from_ip("127.0.0.1".parse().unwrap()), None);
        assert_eq!(from_ip("10.1.2.3".parse().unwrap()), None);
    }
}
