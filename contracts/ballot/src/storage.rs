// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, String, contracttype};

#[contracttype]
#[derive(Clone, Debug)]
pub enum DataKey {
    Admin,
    VotingActive,
    RegisteredVoter(Address),
    Voter(Address),
    /// Binary yes-vote counter (choice index 1).  Kept for backward compat.
    YesVotes,
    /// Binary no-vote counter (choice index 0).  Kept for backward compat.
    NoVotes,
    /// First ledger sequence at which voting is open (inclusive).
    VotingStart,
    /// Last ledger sequence at which voting is open (inclusive).
    VotingEnd,
    /// Running count of total votes cast; used to gate `deregister_voter`.
    TotalVotes,
    /// Minimum voter turnout required before results can be certified (#1126).
    Quorum,
    // ── multi-choice additions (#788) ──────────────────────────────────────
    /// Ordered list of choice labels set at `initialize`.
    Choices,
    /// Vote tally for choice at the given index.
    ChoiceVotes(u32),
    // ── multi-ballot additions (#1127) ─────────────────────────────────────
    /// Number of ballots created so far; also the next ballot id to assign.
    BallotCount,
    /// Title of the ballot with the given id.
    BallotTitle(u32),
    /// Ordered list of choice labels for the ballot with the given id.
    BallotChoices(u32),
    /// First ledger sequence at which the given ballot's voting is open.
    BallotStart(u32),
    /// Last ledger sequence at which the given ballot's voting is open.
    BallotEnd(u32),
    /// Minimum voter turnout required for the given ballot.
    BallotQuorum(u32),
    /// Whether the given ballot is currently active.
    BallotActive(u32),
    /// Running count of total votes cast in the given ballot.
    BallotTotalVotes(u32),
    /// Vote tally for a choice index within the given ballot.
    BallotChoiceVotes(u32, u32),
    /// Whether the given voter has already voted in the given ballot.
    BallotVoter(u32, Address),
}
