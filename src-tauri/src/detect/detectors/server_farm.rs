//! Redirect / spam farms: one IP hosting a wall of fake servers.
//!
//! This is the one listing-wide heuristic. [`farm_ips`] judges the whole
//! listing (discovery also uses it to peel farms off Steam's capped answer);
//! the per-server [`ServerFarm`] detector only reads the verdict the caller
//! put in [`Context::on_farm_ip`](crate::detect::Context::on_farm_ip).

use crate::detect::{Detector, Reason, Subject};

/// Listed endpoints on one IP from which its *signature* is examined.
///
/// Port count alone is not a farm signal. Measured against a complete
/// (farm-excluded) Steam listing: real Latin-American hosts run 10–18
/// community servers on one IP (`45.235.98.52`: "KREEDZ KZ EASY · Nostalgia",
/// "MATA AL TRAIDOR TTT · Nostalgia", …, 107 players), so a bare
/// "10+ ports ⇒ farm" rule hid them. See [`farm_ips`] for what else must hold.
pub const FARM_MIN_ENDPOINTS: usize = 10;

/// Endpoints on one IP that no real host runs: a farm by volume alone.
///
/// The largest real host measured ran 18 servers on one IP; every IP at 64
/// or more was a farm (placeholder, random or cloned names, 98–540 ports).
pub const FARM_VOLUME_ENDPOINTS: usize = 64;

/// One listed endpoint, as the farm classifier sees it.
pub struct ListedEndpoint<'a> {
    pub ip: std::net::Ipv4Addr,
    pub hostname: &'a str,
}

/// IPs that are redirect / spam farms, judged from the whole listing.
///
/// A farm is an IP with at least [`FARM_MIN_ENDPOINTS`] listed servers **and**
/// a farm signature, or at least [`FARM_VOLUME_ENDPOINTS`] servers outright.
/// The signatures, each measured on the live list:
///
/// - **cloned names** — half or more of its servers repeat a name another of
///   its servers already uses ("CS 1.6" ×199, "This is a dummy CS 1.6
///   server!" ×514, five cloned community names over 463 ports);
/// - **random names** — half or more are machine-generated tokens
///   (`g1dLhYJdo8j0YFF8` ×199).
///
/// A real multi-server host has distinct, human names and passes. The
/// advertising farms that clone *other* communities' names with a distinct
/// name per port all claim 255 slots, which the per-server `slots>32` rule
/// already rejects.
pub fn farm_ips<'a>(
    rows: impl IntoIterator<Item = ListedEndpoint<'a>>,
) -> std::collections::HashSet<std::net::Ipv4Addr> {
    use std::collections::{HashMap, HashSet};
    let mut by_ip: HashMap<std::net::Ipv4Addr, Vec<&str>> = HashMap::new();
    for r in rows {
        by_ip.entry(r.ip).or_default().push(r.hostname.trim());
    }
    let farms: HashSet<std::net::Ipv4Addr> = by_ip
        .into_iter()
        .filter(|(ip, names)| {
            let n = names.len();
            if n >= FARM_VOLUME_ENDPOINTS {
                tracing::debug!(%ip, ports = n, "[detect] farm IP: port count alone");
                return true;
            }
            if n < FARM_MIN_ENDPOINTS {
                return false;
            }
            let distinct: HashSet<&str> = names.iter().copied().collect();
            let cloned = n - distinct.len();
            let random = names.iter().filter(|name| is_random_token(name)).count();
            let is_farm = cloned * 2 >= n || random * 2 >= n;
            if is_farm {
                tracing::debug!(%ip, ports = n, cloned, random, "[detect] farm IP: name signature");
            }
            is_farm
        })
        .map(|(ip, _)| ip)
        .collect();
    if !farms.is_empty() {
        tracing::debug!(
            count = farms.len(),
            "[detect] farm IPs identified this pass"
        );
    }
    farms
}

/// A machine-generated hostname: one 12–32 character token of letters and
/// digits that flips between upper case, lower case and digits at least once
/// every three characters (`g1dLhYJdo8j0YFF8`: 11 flips in 16). A human
/// token such as `CS16Server2024` flips rarely (4 in 14), and real names
/// usually have spaces or punctuation anyway.
pub fn is_random_token(name: &str) -> bool {
    let name = name.trim();
    if !(12..=32).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let class = |c: char| {
        if c.is_ascii_digit() {
            0
        } else if c.is_ascii_uppercase() {
            1
        } else {
            2
        }
    };
    let chars: Vec<u8> = name.chars().map(class).collect();
    let flips = chars.windows(2).filter(|w| w[0] != w[1]).count();
    flips * 3 >= name.len()
}

pub struct ServerFarm;

impl Detector for ServerFarm {
    fn reason(&self) -> Reason {
        Reason::ServerFarm
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        s.ctx.on_farm_ip
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn host(ip: [u8; 4], names: &[String]) -> Vec<(Ipv4Addr, String)> {
        names
            .iter()
            .map(|n| (Ipv4Addr::from(ip), n.clone()))
            .collect()
    }

    fn farms(rows: &[(Ipv4Addr, String)]) -> std::collections::HashSet<Ipv4Addr> {
        farm_ips(rows.iter().map(|(ip, n)| ListedEndpoint {
            ip: *ip,
            hostname: n,
        }))
    }

    #[test]
    fn a_real_multi_server_host_is_not_a_farm() {
        // Shaped on 45.235.98.52: many distinct community servers on one IP.
        let names: Vec<String> = [
            "KREEDZ KZ EASY · Nostalgia",
            "MATA AL TRAIDOR TTT · Nostalgia",
            "GUNGAME + FFA + TEAMPLAY · Nostalgia",
            "PUBLICO I · Nostalgia",
            "PUBLICO II · Nostalgia",
            "PUBLICO III + RESPAWN · Nostalgia",
            "AUTOMIX 1 · Nostalgia",
            "AUTOMIX 2 · Nostalgia",
            "DEATHMATCH · Nostalgia",
            "SURF · Nostalgia",
            "JAILBREAK · Nostalgia",
            "ZOMBIE · Nostalgia",
            "HNS · Nostalgia",
            "DEATHRUN · Nostalgia",
            "SOCCERJAM · Nostalgia",
            "BASEBUILDER · Nostalgia",
            "AIM MAPS · Nostalgia",
            "AWP ONLY · Nostalgia",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(farms(&host([45, 235, 98, 52], &names)).is_empty());
    }

    #[test]
    fn cloned_placeholder_names_are_a_farm() {
        let names = vec!["CS 1.6".to_string(); 29];
        assert_eq!(farms(&host([64, 176, 68, 189], &names)).len(), 1);
    }

    #[test]
    fn random_token_names_are_a_farm() {
        let names: Vec<String> = (0..20).map(|i| format!("g1dLhYJdo8j0YF{i:02}")).collect();
        assert!(names.iter().all(|n| is_random_token(n)));
        assert_eq!(farms(&host([5, 188, 88, 19], &names)).len(), 1);
    }

    #[test]
    fn volume_alone_is_a_farm_and_small_hosts_never_are() {
        let many: Vec<String> = (0..FARM_VOLUME_ENDPOINTS)
            .map(|i| format!("Server {i}"))
            .collect();
        assert_eq!(farms(&host([2, 27, 105, 168], &many)).len(), 1);
        // Below the threshold even identical names are left to other rules.
        let few = vec!["CS 1.6".to_string(); FARM_MIN_ENDPOINTS - 1];
        assert!(farms(&host([9, 9, 9, 9], &few)).is_empty());
    }

    #[test]
    fn human_names_are_not_random_tokens() {
        for name in [
            "Wolves | Dust TOP #2",
            "Techline",
            "PROCS.LT",
            "1shot2kill.pl",
            "CS16Server2024",
        ] {
            assert!(!is_random_token(name), "{name}");
        }
    }
}
