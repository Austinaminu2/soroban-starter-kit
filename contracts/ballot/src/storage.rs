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
    // ── multi-choice additions (#788) ──────────────────────────────────────
    /// Ordered list of choice labels set at `initialize`.
    Choices,
    /// Vote tally for choice at the given index.
    ChoiceVotes(u32),
}
