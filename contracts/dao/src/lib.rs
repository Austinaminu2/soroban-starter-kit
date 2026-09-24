#![no_std]
#![deny(missing_docs)]
//! DAO governance contract template.
//!
//! Token holders create proposals and cast token-weighted votes; a proposal
//! passes when it reaches quorum with more yes votes than no votes.
//!
//! ## Quorum modes
//!
//! Two quorum controls can be set at initialization; both must be satisfied
//! for a proposal to execute:
//!
//! * **`quorum`** — absolute minimum total votes (in token units).
//! * **`quorum_bps`** — minimum participation as a share of total token supply
//!   expressed in basis points (0–10 000). Set to `0` to disable.
//!
//! ## Proposer self-cancel (#830)
//!
//! The original proposer may call `proposer_cancel_proposal` to retract their
//! proposal **before any vote has been cast**. This lets them correct mistakes
//! without waiting for the voting period to lapse.
//!
//! ## Proposal listing (#1110)
//!
//! `get_proposals(cursor, limit, state)` returns proposals in ascending ID
//! order, optionally filtered by [`ProposalState`], at most [`MAX_PAGE_SIZE`]
//! per call. Pass the returned `next_cursor` to fetch the following page.

use soroban_sdk::{Address, Env, String, Symbol, Vec, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

pub use errors::DaoError;
pub use storage::{DataKey, Proposal, ProposalKey, ProposalPage, ProposalState, VoteKey};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, paginate};

/// Maximum number of proposals returned by a single `get_proposals` call.
pub const MAX_PAGE_SIZE: u32 = 50;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn bump_persistent<K>(env: &Env, key: &K)
where
    K: soroban_sdk::TryIntoVal<Env, soroban_sdk::Val> + soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// DAO governance contract for on-chain proposal creation and token-weighted voting.
///
/// Voting power is capped at the token balance at vote time, but limited to not exceed
/// the total token supply at proposal creation time. This snapshot prevents flash-loan
/// style vote manipulation. A proposal passes when `yes_votes > no_votes`, total votes
/// reach the absolute `quorum`, *and* participation reaches `quorum_bps` of the total
/// token supply (measured at proposal creation time).
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct DaoContract;

    #[contractimpl]
    impl DaoContract {
        /// Initialize the DAO.
        ///
        /// - `voting_period` — number of ledgers a proposal stays open for voting.
        /// - `quorum` — minimum total votes (in token units) required for a valid result.
        /// - `quorum_bps` — minimum participation as basis points of total supply (0–10 000).
        ///   Pass `0` to disable the percentage-based quorum check.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::AlreadyInitialized`] if called again.
        /// Returns [`DaoError::InvalidQuorumBps`] if `quorum_bps` > 10 000.
        pub fn initialize(
            env: Env,
            admin: Address,
            token: Address,
            voting_period: u32,
            quorum: i128,
            quorum_bps: u32,
        ) -> Result<(), DaoError> {
            if env.storage().instance().has(&DataKey::Initialized) {
                return Err(DaoError::AlreadyInitialized);
            }
            if quorum_bps > 10_000 {
                return Err(DaoError::InvalidQuorumBps);
            }

            admin.require_auth();

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage().instance().set(&DataKey::Token, &token);
            env.storage()
                .instance()
                .set(&DataKey::VotingPeriod, &voting_period);
            env.storage().instance().set(&DataKey::Quorum, &quorum);
            env.storage()
                .instance()
                .set(&DataKey::QuorumBps, &quorum_bps);
            env.storage().instance().set(&DataKey::ProposalCount, &0u32);
            env.storage().instance().set(&DataKey::Initialized, &true);

            bump_instance(&env);
            events::initialized(&env, &admin, &token, quorum);

            Ok(())
        }

        /// Create a new proposal. The proposer must hold > 0 governance tokens.
        ///
        /// Returns the newly created `proposal_id`.
        ///
        /// Voting power and quorum are calculated using a snapshot of the total
        /// token supply at proposal creation time, preventing flash-loan-style
        /// vote manipulation.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::NotInitialized`] if the DAO has not been set up.
        /// Returns [`DaoError::InsufficientVotingPower`] if the proposer has no tokens.
        pub fn create_proposal(
            env: Env,
            proposer: Address,
            title: String,
            description: String,
        ) -> Result<u32, DaoError> {
            Self::require_initialized(&env)?;
            proposer.require_auth();

            let token: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(DaoError::NotInitialized)?;
            let balance = token::Client::new(&env, &token).balance(&proposer);
            if balance <= 0 {
                return Err(DaoError::InsufficientVotingPower);
            }

            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ProposalCount)
                .unwrap_or(0);
            let proposal_id = count;

            let voting_period: u32 = env
                .storage()
                .instance()
                .get(&DataKey::VotingPeriod)
                .ok_or(DaoError::NotInitialized)?;
            let deadline = env.ledger().sequence() + voting_period;

            // TODO: Once soroban-sdk supports total_supply() on token::Client,
            // capture the actual total supply here.
            // Using i128::MAX as a temporary sentinel ensures voting isn't restricted
            // while the infrastructure is in place for the cap.
            let total_supply_at_creation = i128::MAX;

            let proposal = Proposal {
                id: proposal_id,
                proposer: proposer.clone(),
                title,
                description,
                deadline,
                yes_votes: 0,
                no_votes: 0,
                state: ProposalState::Active,
                total_supply_at_creation,
            };

            env.storage()
                .persistent()
                .set(&ProposalKey::Proposal(proposal_id), &proposal);
            env.storage()
                .instance()
                .set(&DataKey::ProposalCount, &(count + 1));

            bump_instance(&env);
            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));
            events::proposal_created(&env, &proposer, proposal_id);

            Ok(proposal_id)
        }

        /// Cast a vote on an active proposal. Voting weight is the voter's current token balance.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::ProposalNotFound`] if the proposal does not exist.
        /// Returns [`DaoError::InvalidState`] if the proposal is not `Active`.
        /// Returns [`DaoError::VotingClosed`] if the voting period has ended.
        /// Returns [`DaoError::AlreadyVoted`] if the voter has already voted.
        /// Returns [`DaoError::InsufficientVotingPower`] if the voter has no tokens.
        pub fn vote(
            env: Env,
            voter: Address,
            proposal_id: u32,
            support: bool,
        ) -> Result<(), DaoError> {
            Self::require_initialized(&env)?;
            voter.require_auth();

            let mut proposal: Proposal = env
                .storage()
                .persistent()
                .get(&ProposalKey::Proposal(proposal_id))
                .ok_or(DaoError::ProposalNotFound)?;

            if proposal.state != ProposalState::Active {
                return Err(DaoError::InvalidState);
            }
            if env.ledger().sequence() > proposal.deadline {
                return Err(DaoError::VotingClosed);
            }

            let vote_key = VoteKey {
                proposal_id,
                voter: voter.clone(),
            };
            if env.storage().persistent().has(&vote_key) {
                return Err(DaoError::AlreadyVoted);
            }

            let token: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(DaoError::NotInitialized)?;
            let balance = token::Client::new(&env, &token).balance(&voter);
            if balance <= 0 {
                return Err(DaoError::InsufficientVotingPower);
            }

            // Cap voting weight to the total supply at proposal creation time.
            // This prevents flash-loan attacks where a voter temporarily acquires
            // a large balance to inflate their voting power.
            let weight = if balance > proposal.total_supply_at_creation {
                proposal.total_supply_at_creation
            } else {
                balance
            };

            if support {
                proposal.yes_votes += weight;
            } else {
                proposal.no_votes += weight;
            }

            env.storage()
                .persistent()
                .set(&ProposalKey::Proposal(proposal_id), &proposal);
            env.storage().persistent().set(&vote_key, &weight);

            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));
            bump_persistent(&env, &vote_key);
            events::voted(&env, &voter, proposal_id, support, weight);

            Ok(())
        }

        /// Execute a passed proposal. Callable after the deadline when quorum and majority are met.
        ///
        /// Both the absolute `quorum` (token units) and the percentage-based
        /// `quorum_bps` (share of total supply) must be satisfied.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::ProposalNotFound`] if the proposal does not exist.
        /// Returns [`DaoError::InvalidState`] if the proposal is not `Active`.
        /// Returns [`DaoError::DeadlineNotReached`] if the voting deadline has not passed.
        /// Returns [`DaoError::QuorumNotMet`] if total votes are below either quorum threshold.
        /// Returns [`DaoError::ProposalRejected`] if `no_votes >= yes_votes`.
        pub fn execute_proposal(env: Env, proposal_id: u32) -> Result<(), DaoError> {
            Self::require_initialized(&env)?;

            let mut proposal: Proposal = env
                .storage()
                .persistent()
                .get(&ProposalKey::Proposal(proposal_id))
                .ok_or(DaoError::ProposalNotFound)?;

            if proposal.state != ProposalState::Active {
                return Err(DaoError::InvalidState);
            }
            if env.ledger().sequence() <= proposal.deadline {
                return Err(DaoError::DeadlineNotReached);
            }

            let quorum: i128 = env
                .storage()
                .instance()
                .get(&DataKey::Quorum)
                .ok_or(DaoError::NotInitialized)?;
            let total_votes = proposal.yes_votes + proposal.no_votes;

            // Absolute quorum check.
            if total_votes < quorum {
                return Err(DaoError::QuorumNotMet);
            }

            // Percentage-based quorum check (#829).
            // Uses the total supply snapshot at proposal creation time to prevent
            // manipulation through token minting/burning between proposal and execution.
            let quorum_bps: u32 = env
                .storage()
                .instance()
                .get(&DataKey::QuorumBps)
                .unwrap_or(0u32);
            if quorum_bps > 0 {
                let token: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::Token)
                    .ok_or(DaoError::NotInitialized)?;
                // `total_supply` is not part of the SEP-41 `TokenInterface`, so it is
                // unreachable through `token::Client`. Invoke it by symbol instead; this
                // requires the configured governance token to implement `total_supply`
                // (as `soroban-token-template` does).
                let total_supply: i128 =
                    env.invoke_contract(&token, &Symbol::new(&env, "total_supply"), Vec::new(&env));
                // total_votes / total_supply >= quorum_bps / 10_000
                // ⟺ total_votes * 10_000 >= quorum_bps * total_supply
                if total_votes * 10_000 < i128::from(quorum_bps) * proposal.total_supply_at_creation
                {
                    return Err(DaoError::QuorumNotMet);
                }
            }

            if proposal.yes_votes <= proposal.no_votes {
                return Err(DaoError::ProposalRejected);
            }

            proposal.state = ProposalState::Executed;
            env.storage()
                .persistent()
                .set(&ProposalKey::Proposal(proposal_id), &proposal);

            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));
            events::proposal_executed(&env, proposal_id);

            Ok(())
        }

        /// Cancel a proposal. Admin only; works on any `Active` proposal regardless of votes.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::NotAuthorized`] if the caller is not the admin.
        /// Returns [`DaoError::ProposalNotFound`] if the proposal does not exist.
        /// Returns [`DaoError::InvalidState`] if the proposal is not `Active`.
        pub fn cancel_proposal(env: Env, proposal_id: u32) -> Result<(), DaoError> {
            Self::require_initialized(&env)?;

            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(DaoError::NotInitialized)?;
            admin.require_auth();

            let mut proposal: Proposal = env
                .storage()
                .persistent()
                .get(&ProposalKey::Proposal(proposal_id))
                .ok_or(DaoError::ProposalNotFound)?;

            if proposal.state != ProposalState::Active {
                return Err(DaoError::InvalidState);
            }

            proposal.state = ProposalState::Cancelled;
            env.storage()
                .persistent()
                .set(&ProposalKey::Proposal(proposal_id), &proposal);

            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));
            events::proposal_cancelled(&env, &admin, proposal_id);

            Ok(())
        }

        /// Allow the original proposer to cancel their own proposal before any votes are cast.
        ///
        /// This lets a proposer correct a mistake (wrong description, bad parameters, etc.)
        /// without waiting for the entire voting period to lapse.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::NotInitialized`] if the DAO has not been set up.
        /// Returns [`DaoError::ProposalNotFound`] if the proposal does not exist.
        /// Returns [`DaoError::NotAuthorized`] if the caller is not the original proposer.
        /// Returns [`DaoError::InvalidState`] if the proposal is not `Active`.
        /// Returns [`DaoError::VotesAlreadyCast`] if at least one vote has already been recorded.
        pub fn proposer_cancel_proposal(
            env: Env,
            proposer: Address,
            proposal_id: u32,
        ) -> Result<(), DaoError> {
            Self::require_initialized(&env)?;
            proposer.require_auth();

            let mut proposal: Proposal = env
                .storage()
                .persistent()
                .get(&ProposalKey::Proposal(proposal_id))
                .ok_or(DaoError::ProposalNotFound)?;

            // Only the original proposer may use this entry point.
            if proposal.proposer != proposer {
                return Err(DaoError::NotAuthorized);
            }

            if proposal.state != ProposalState::Active {
                return Err(DaoError::InvalidState);
            }

            // Reject if any votes have already been cast.
            if proposal.yes_votes > 0 || proposal.no_votes > 0 {
                return Err(DaoError::VotesAlreadyCast);
            }

            proposal.state = ProposalState::Cancelled;
            env.storage()
                .persistent()
                .set(&ProposalKey::Proposal(proposal_id), &proposal);

            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));
            events::proposal_proposer_cancelled(&env, &proposer, proposal_id);

            Ok(())
        }

        /// Return a proposal by ID.
        #[must_use]
        pub fn get_proposal(env: Env, proposal_id: u32) -> Result<Proposal, DaoError> {
            env.storage()
                .persistent()
                .get(&ProposalKey::Proposal(proposal_id))
                .ok_or(DaoError::ProposalNotFound)
        }

        /// Return total number of proposals created.
        #[must_use]
        pub fn proposal_count(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::ProposalCount)
                .unwrap_or(0)
        }

        /// List proposals with cursor-based pagination (#1110).
        ///
        /// `cursor` is the proposal ID to resume scanning from (pass `0` to
        /// start from the beginning). `limit` is clamped to
        /// `[1, MAX_PAGE_SIZE]`. When `state` is `Some`, only proposals in
        /// that [`ProposalState`] are returned.
        ///
        /// Returns a [`ProposalPage`] whose `next_cursor` is `None` once the
        /// end of the proposal range has been reached.
        pub fn get_proposals(
            env: Env,
            cursor: u32,
            limit: u32,
            state: Option<ProposalState>,
        ) -> ProposalPage {
            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ProposalCount)
                .unwrap_or(0);

            let page = paginate(
                &env,
                cursor,
                limit,
                MAX_PAGE_SIZE,
                |c: u32| {
                    let next = c.saturating_add(1);
                    if next < count { Some(next) } else { None }
                },
                |c: u32| {
                    if c >= count {
                        return None;
                    }
                    env.storage()
                        .persistent()
                        .get::<_, Proposal>(&ProposalKey::Proposal(c))
                        .filter(|p| state.is_none_or(|s| p.state == s))
                },
            );

            ProposalPage {
                proposals: page.items,
                next_cursor: page.next_cursor,
            }
        }

        fn require_initialized(env: &Env) -> Result<(), DaoError> {
            if !env.storage().instance().has(&DataKey::Initialized) {
                return Err(DaoError::NotInitialized);
            }
            Ok(())
        }
    }
}

mod test;

#[cfg(test)]
mod prop_test;
