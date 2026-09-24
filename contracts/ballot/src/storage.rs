// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, BytesN, String, Vec, contracttype};

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
    // ── quadratic voting additions (#1123) ─────────────────────────────────
    /// Whether quadratic voting mode is enabled for this ballot.
    Quadratic,
    // ── ranked-choice additions (#1124) ────────────────────────────────────
    /// Ordered preference ranking submitted by a voter (most preferred first).
    RankedVote(Address),
    /// Number of ranked-choice ballots cast; used to size elimination rounds.
    RankedVoteCount,
    // ── commit-reveal additions (#1125) ────────────────────────────────────
    /// Whether commit-reveal secret voting mode is enabled for this ballot.
    CommitReveal,
    /// Commitment hash `hash(voter ++ choice ++ salt)` submitted during the
    /// commit phase, keyed by voter.  The choice stays secret until reveal.
    Commitment(Address),
    /// Whether a voter has revealed their committed ballot.
    Revealed(Address),
    /// Number of commitments submitted; used to gate the reveal phase.
    CommitCount,
}

/// A single instant-runoff elimination round result.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RoundResult {
    /// Per-choice tallies for this round, indexed by choice index.
    pub tallies: Vec<u32>,
    /// Choice eliminated at the end of this round, or `None` for the final
    /// round in which a majority winner was found.
    pub eliminated: Option<u32>,
    /// Choice that reached a majority in this round, if any.
    pub winner: Option<u32>,
}

/// A stored commit-reveal commitment for a single voter.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Commitment {
    /// `hash(voter ++ choice ++ salt)` submitted during the commit phase.
    pub hash: BytesN<32>,
    /// Ledger sequence at which the commitment was recorded.
    pub committed_at: u32,
}
