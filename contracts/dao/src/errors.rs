use soroban_sdk::contracterror;
use soroban_common::impl_display_error;

// `#[contracterror]` generates undocumented public associated items.
#[allow(missing_docs)]
#[contracterror]
#[derive(Clone, Copy, Debug)]
pub enum DaoError {
    NotAuthorized = 1,
    AlreadyInitialized = 2,
    NotInitialized = 3,
    ProposalNotFound = 4,
    InvalidState = 5,
    DeadlineNotReached = 6,
    AlreadyVoted = 7,
    QuorumNotMet = 8,
    ProposalRejected = 9,
    InsufficientVotingPower = 10,
    /// Proposer's token balance is below the required proposal bond (issue #1106).
    InsufficientBondBalance = 11,
    /// Action dispatch via `env.invoke_contract` failed (issue #1108).
    ActionFailed = 12,
}

impl_display_error!(
    DaoError,
    NotAuthorized           => "not authorized",
    AlreadyInitialized      => "already initialized",
    NotInitialized          => "not initialized",
    ProposalNotFound        => "proposal not found",
    InvalidState            => "invalid proposal state",
    DeadlineNotReached      => "voting deadline not yet reached",
    AlreadyVoted            => "already voted on this proposal",
    QuorumNotMet            => "quorum not met",
    ProposalRejected        => "proposal rejected by majority",
    InsufficientVotingPower => "insufficient voting power",
    InsufficientBondBalance => "insufficient balance for proposal bond",
    ActionFailed            => "proposal action dispatch failed",
);

#[cfg(test)]
mod tests {
    extern crate std;

    use super::DaoError;
    use std::format;
    use std::string::String;

    #[allow(clippy::as_conversions)]
    fn render_error_code_snapshot() -> String {
        format!(
            "\
DaoError::NotAuthorized = {}\n\
DaoError::AlreadyInitialized = {}\n\
DaoError::NotInitialized = {}\n\
DaoError::ProposalNotFound = {}\n\
DaoError::InvalidState = {}\n\
DaoError::DeadlineNotReached = {}\n\
DaoError::AlreadyVoted = {}\n\
DaoError::QuorumNotMet = {}\n\
DaoError::ProposalRejected = {}\n\
DaoError::InsufficientVotingPower = {}\n\
DaoError::InsufficientBondBalance = {}\n\
DaoError::ActionFailed = {}\n",
            DaoError::NotAuthorized as u32,
            DaoError::AlreadyInitialized as u32,
            DaoError::NotInitialized as u32,
            DaoError::ProposalNotFound as u32,
            DaoError::InvalidState as u32,
            DaoError::DeadlineNotReached as u32,
            DaoError::AlreadyVoted as u32,
            DaoError::QuorumNotMet as u32,
            DaoError::ProposalRejected as u32,
            DaoError::InsufficientVotingPower as u32,
            DaoError::InsufficientBondBalance as u32,
            DaoError::ActionFailed as u32,
        )
    }

    #[test]
    fn dao_error_codes_match_snapshot() {
        assert_eq!(
            render_error_code_snapshot(),
            include_str!("../snapshots/error_codes.snap")
        );
    }
}
