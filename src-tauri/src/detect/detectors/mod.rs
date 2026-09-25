//! The heuristics, one per file, and the registry [`analyze`](super::analyze)
//! runs.

use super::Detector;

pub mod blocked_name;
pub mod bot_flood;
pub mod empty_gamedir;
pub mod empty_hostname;
pub mod empty_map;
pub mod foreign_address;
pub mod impossible_players;
pub mod name_churn;
pub mod repeated_hostname;
pub mod server_farm;
pub mod slot_cap;

/// Every detector, in the order its reason appears in an [`Analysis`].
///
/// [`Analysis`]: super::Analysis
pub const ALL: &[&dyn Detector] = &[
    // Hostname text
    &empty_hostname::EmptyHostname,
    &blocked_name::BlockedName,
    &foreign_address::ForeignAddress,
    // Player counts
    &impossible_players::ImpossiblePlayers,
    &slot_cap::SlotCap,
    &bot_flood::BotFlood,
    // Identity history
    &name_churn::NameChurn,
    // Map / gamedir
    &empty_map::EmptyMap,
    &empty_gamedir::EmptyGameDir,
    // Listing-wide verdicts supplied by the caller
    &server_farm::ServerFarm,
    &repeated_hostname::RepeatedHostname,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Reason;

    /// A reason no detector raises can never fire; two detectors raising one
    /// reason would duplicate it in an analysis.
    #[test]
    fn every_reason_has_exactly_one_detector() {
        for reason in Reason::ALL {
            let n = ALL.iter().filter(|d| d.reason() == reason).count();
            assert_eq!(n, 1, "{reason:?} is raised by {n} detectors");
        }
        assert_eq!(ALL.len(), Reason::ALL.len());
    }
}
