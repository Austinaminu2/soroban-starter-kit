// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, String, Symbol, Val, Vec, contracttype};

/// Instance-storage keys (contract-level state).
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Token,
    VotingPeriod,
    Quorum,
    ProposalCount,
    Initialized,
    /// Configurable bond amount escrowed per proposal (issue #1106).
    ProposalBond,
    /// Adaptive quorum parameters (issue #1107).
    MinQuorumBps,
    MaxQuorumBps,
    /// Number of past proposals tracked for EMA smoothing (issue #1107).
    QuorumEmaWindow,
    /// Current smoothed quorum BPS (0–10_000). Stored as u32.
    QuorumEmaBps,
    /// Number of proposals included in the EMA so far.
    QuorumEmaCount,
}

/// Persistent-storage keys (per-proposal and per-vote data).
#[contracttype]
#[derive(Clone)]
pub enum ProposalKey {
    Proposal(u32),
}

/// Composite key for vote deduplication.
#[contracttype]
#[derive(Clone)]
pub struct VoteKey {
    pub proposal_id: u32,
    pub voter: Address,
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProposalState {
    Active = 0,
    Executed = 1,
    Cancelled = 2,
}

impl core::fmt::Display for ProposalState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            ProposalState::Active => "active",
            ProposalState::Executed => "executed",
            ProposalState::Cancelled => "cancelled",
        })
    }
}

/// A governance proposal.
///
/// `action_target`, `action_function`, and `action_args` are optional; when
/// `action_target` is `Some`, `execute_proposal` will dispatch the call via
/// `env.invoke_contract` (issue #1108).
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u32,
    pub proposer: Address,
    pub title: String,
    pub description: String,
    pub deadline: u32,
    pub yes_votes: i128,
    pub no_votes: i128,
    pub state: ProposalState,
    // ── Executable action payload (issue #1108) ──────────────────────────────
    /// Optional target contract to call on execution.
    pub action_target: Option<Address>,
    /// Function name to invoke on the target contract.
    pub action_function: Option<Symbol>,
    /// Arguments forwarded to the target function.
    pub action_args: Option<Vec<Val>>,
    // ── Bond tracking (issue #1106) ──────────────────────────────────────────
    /// Token amount escrowed by the proposer at submission time.
    pub bond_amount: i128,
}
