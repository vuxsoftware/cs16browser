//! Hostname advertises a different server's IPv4 address.

use crate::detect::{Detector, Reason, Subject};
use std::net::Ipv4Addr;

/// Every public dotted-quad IPv4 address written in `text`.
///
/// Real servers often print their *own* address ("IP: 1.2.3.4:27015"), which
/// is harmless; the caller compares against the server's own IP. A LAN or
/// otherwise non-routable address cannot send an Internet player anywhere,
/// so it is not a signpost.
fn advertised_ips(text: &str) -> impl Iterator<Item = Ipv4Addr> + '_ {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter_map(|tok| tok.trim_matches('.').parse::<Ipv4Addr>().ok())
        .filter(is_public)
}

/// Routable on the Internet (`Ipv4Addr::is_global` is still unstable).
fn is_public(ip: &Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        // Carrier-grade NAT, 100.64.0.0/10.
        || (a == 100 && (64..128).contains(&b)))
}

pub struct ForeignAddress;

impl Detector for ForeignAddress {
    fn reason(&self) -> Reason {
        Reason::ForeignAddress
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        advertised_ips(&s.info.hostname).any(|ip| ip != s.info.endpoint.ip)
    }
}

#[cfg(test)]
mod tests {
    use crate::detect::testing::sample;
    use crate::detect::{analyze, Context, Reason};

    #[test]
    fn advertising_another_address_is_a_hard_redirect() {
        let s = sample("Join us! Connect to 5.6.7.8:27015 | Free VIP");
        let a = analyze(&s, &Context::default());
        assert!(a.is_fake());
        assert!(a.reasons.contains(&Reason::ForeignAddress));
    }

    /// Measured: every real row this fired on was a moved server pointing
    /// at its new address. The old listing is a signpost, not a server.
    #[test]
    fn a_moved_server_notice_is_a_signpost() {
        let a = analyze(
            &sample("[KGB] SERVER MOVED / SERVER PREMJESTEN -> 82.29.125.131:27016"),
            &Context::default(),
        );
        assert!(a.reasons.contains(&Reason::ForeignAddress));
    }

    #[test]
    fn a_lan_address_is_not_a_signpost() {
        for name in [
            "LAN party 192.168.1.10",
            "Office 10.0.0.5:27015",
            "CGNAT 100.64.1.1",
        ] {
            let a = analyze(&sample(name), &Context::default());
            assert!(a.reasons.is_empty(), "{name:?}: {:?}", a.reasons);
        }
    }

    #[test]
    fn printing_its_own_address_is_fine() {
        // `sample` lives on 1.2.3.4.
        let s = sample("Public #1 | IP: 1.2.3.4:27015");
        assert!(!analyze(&s, &Context::default()).is_fake());
    }
}
