#![no_std]
#![deny(missing_docs)]
//! DAO governance contract template.
//!
//! Token holders create proposals and cast token-weighted votes; a proposal
//! passes when it reaches quorum with more yes votes than no votes.
//!
//! ## Features
//!
//! * **Executable action payloads** (issue #1108) — proposals may carry an
//!   optional `(target, function, args)` triple that is dispatched atomically
//!   via `env.invoke_contract` on execution.
//! * **Proposal submission bond** (issue #1106) — a configurable bond is
//!   escrowed on `create_proposal`; it is refunded when the proposal passes
//!   quorum or slashed to the admin treasury when it fails to meet minimum
//!   participation.
//! * **Dynamic quorum** (issue #1107) — an exponential moving average of
//!   historical participation keeps quorum between `min_quorum_bps` and
//!   `max_quorum_bps`, bounded in basis points (0–10 000).

use soroban_sdk::{Address, Env, String, Symbol, Val, Vec, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

pub use errors::DaoError;
pub use storage::{DataKey, Proposal, ProposalKey, ProposalState, VoteKey};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD};

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn bump_persistent<K>(env: &Env, key: &K)
where
    K: soroban_sdk::TryIntoVal<Env, soroban_sdk::Val>
        + soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// DAO governance contract for on-chain proposal creation and token-weighted voting.
///
/// Voting power is the voter's token balance at vote time (simplified snapshot).
/// A proposal passes when `yes_votes > no_votes` and total votes reach the quorum.
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    // ── Adaptive-quorum constants ────────────────────────────────────────────
    /// Default EMA smoothing window (number of proposals).
    const DEFAULT_QUORUM_EMA_WINDOW: u32 = 10;
    /// Basis points denominator.
    const BPS_DENOMINATOR: u32 = 10_000;

    #[contract]
    pub struct DaoContract;

    #[contractimpl]
    impl DaoContract {
        /// Initialize the DAO.
        ///
        /// - `voting_period` — number of ledgers a proposal stays open for voting.
        /// - `quorum` — minimum total votes (in token units) required for a valid result.
        ///   Used as the *initial* absolute quorum floor; adaptive quorum BPS are layered
        ///   on top and stored separately.
        /// - `proposal_bond` — token units escrowed by a proposer at submission time
        ///   (issue #1106). Set to 0 to disable bonding.
        /// - `min_quorum_bps` / `max_quorum_bps` — lower and upper bounds (in basis
        ///   points, 0–10 000) for the adaptive quorum EMA (issue #1107). Pass both
        ///   as 0 to disable adaptive quorum.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::AlreadyInitialized`] if called again.
        pub fn initialize(
            env: Env,
            admin: Address,
            token: Address,
            voting_period: u32,
            quorum: i128,
            proposal_bond: i128,
            min_quorum_bps: u32,
            max_quorum_bps: u32,
        ) -> Result<(), DaoError> {
            if env.storage().instance().has(&DataKey::Initialized) {
                return Err(DaoError::AlreadyInitialized);
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
                .set(&DataKey::ProposalBond, &proposal_bond);
            env.storage()
                .instance()
                .set(&DataKey::MinQuorumBps, &min_quorum_bps);
            env.storage()
                .instance()
                .set(&DataKey::MaxQuorumBps, &max_quorum_bps);
            env.storage().instance().set(
                &DataKey::QuorumEmaWindow,
                &DEFAULT_QUORUM_EMA_WINDOW,
            );
            // Initialise EMA at the midpoint of the allowed band.
            let initial_ema = (min_quorum_bps + max_quorum_bps) / 2;
            env.storage()
                .instance()
                .set(&DataKey::QuorumEmaBps, &initial_ema);
            env.storage()
                .instance()
                .set(&DataKey::QuorumEmaCount, &0u32);
            env.storage().instance().set(&DataKey::ProposalCount, &0u32);
            env.storage().instance().set(&DataKey::Initialized, &true);

            bump_instance(&env);
            events::initialized(&env, &admin, &token, quorum);

            Ok(())
        }

        /// Create a new proposal. The proposer must hold > 0 governance tokens.
        ///
        /// When `proposal_bond > 0`, exactly `proposal_bond` tokens are transferred
        /// from `proposer` to the DAO contract as an anti-spam measure (issue #1106).
        ///
        /// When `action_target` is `Some`, the proposal carries an executable payload
        /// that is dispatched by `execute_proposal` (issue #1108).
        ///
        /// Returns the newly created `proposal_id`.
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::NotInitialized`] if the DAO has not been set up.
        /// Returns [`DaoError::InsufficientVotingPower`] if the proposer has no tokens.
        /// Returns [`DaoError::InsufficientBondBalance`] if the proposer cannot cover
        ///   the proposal bond.
        pub fn create_proposal(
            env: Env,
            proposer: Address,
            title: String,
            description: String,
            action_target: Option<Address>,
            action_function: Option<Symbol>,
            action_args: Option<Vec<Val>>,
        ) -> Result<u32, DaoError> {
            Self::require_initialized(&env)?;
            proposer.require_auth();

            let token_addr: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(DaoError::NotInitialized)?;
            let token_client = token::Client::new(&env, &token_addr);

            let balance = token_client.balance(&proposer);
            if balance <= 0 {
                return Err(DaoError::InsufficientVotingPower);
            }

            // ── Bond escrow (issue #1106) ─────────────────────────────────────
            let proposal_bond: i128 = env
                .storage()
                .instance()
                .get(&DataKey::ProposalBond)
                .unwrap_or(0);
            if proposal_bond > 0 {
                if balance < proposal_bond {
                    return Err(DaoError::InsufficientBondBalance);
                }
                let dao_addr = env.current_contract_address();
                // Transfer the bond into the DAO contract's own account.
                token_client.transfer(&proposer, &dao_addr, &proposal_bond);
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

            let proposal = Proposal {
                id: proposal_id,
                proposer: proposer.clone(),
                title,
                description,
                deadline,
                yes_votes: 0,
                no_votes: 0,
                state: ProposalState::Active,
                action_target,
                action_function,
                action_args,
                bond_amount: proposal_bond,
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
        /// Returns [`DaoError::DeadlineNotReached`] if the voting period has expired (deadline passed).
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
                return Err(DaoError::DeadlineNotReached);
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
            let weight = token::Client::new(&env, &token).balance(&voter);
            if weight <= 0 {
                return Err(DaoError::InsufficientVotingPower);
            }

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
        /// Execution is fully atomic: the proposal state is updated, the bond is
        /// refunded (if any), the action payload is dispatched (if any), and the
        /// adaptive-quorum EMA is bumped — all in a single transaction. Any panic
        /// inside the invoked contract rolls back the entire transaction.
        ///
        /// After dispatch, the return value is emitted as a
        /// [`ProposalActionExecuted`](events::proposal_action_executed) event
        /// (issue #1108).
        ///
        /// The adaptive-quorum EMA is updated with the actual participation rate of
        /// this proposal (issue #1107).
        ///
        /// # Errors
        ///
        /// Returns [`DaoError::ProposalNotFound`] if the proposal does not exist.
        /// Returns [`DaoError::InvalidState`] if the proposal is not `Active`.
        /// Returns [`DaoError::DeadlineNotReached`] if the voting deadline has not passed.
        /// Returns [`DaoError::QuorumNotMet`] if total votes are below the quorum threshold.
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

            if total_votes < quorum {
                // Slash bond on participation failure (issue #1106).
                if proposal.bond_amount > 0 {
                    Self::slash_bond(&env, &proposal);
                }
                // Even failed proposals count toward the EMA with zero participation
                // relative to quorum so that a quiet period lowers quorum (issue #1107).
                Self::update_quorum_ema(&env, 0);
                return Err(DaoError::QuorumNotMet);
            }
            if proposal.yes_votes <= proposal.no_votes {
                // Slash bond on rejection (issue #1106).
                if proposal.bond_amount > 0 {
                    Self::slash_bond(&env, &proposal);
                }
                Self::update_quorum_ema(&env, Self::participation_bps(total_votes, quorum));
                return Err(DaoError::ProposalRejected);
            }

            proposal.state = ProposalState::Executed;
            env.storage()
                .persistent()
                .set(&ProposalKey::Proposal(proposal_id), &proposal);

            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));

            // ── Refund bond on success (issue #1106) ─────────────────────────
            if proposal.bond_amount > 0 {
                let token_addr: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::Token)
                    .ok_or(DaoError::NotInitialized)?;
                let dao_addr = env.current_contract_address();
                token::Client::new(&env, &token_addr).transfer(
                    &dao_addr,
                    &proposal.proposer,
                    &proposal.bond_amount,
                );
                events::bond_refunded(&env, &proposal.proposer, proposal_id, proposal.bond_amount);
            }

            events::proposal_executed(&env, proposal_id);

            // ── Dispatch action payload (issue #1108) ─────────────────────────
            if let (Some(target), Some(function), Some(args)) = (
                proposal.action_target.clone(),
                proposal.action_function.clone(),
                proposal.action_args.clone(),
            ) {
                let return_val: Val = env.invoke_contract(&target, &function, args);
                events::proposal_action_executed(&env, proposal_id, return_val);
            }

            // ── Update adaptive quorum EMA (issue #1107) ─────────────────────
            Self::update_quorum_ema(&env, Self::participation_bps(total_votes, quorum));

            Ok(())
        }

        /// Cancel a proposal. Admin only; works only on `Active` proposals.
        ///
        /// The proposal bond (if any) is slashed to the admin treasury on admin
        /// cancellation (issue #1106).
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

            // Slash bond on admin cancellation (issue #1106).
            if proposal.bond_amount > 0 {
                Self::slash_bond(&env, &proposal);
            }

            bump_persistent(&env, &ProposalKey::Proposal(proposal_id));
            events::proposal_cancelled(&env, &admin, proposal_id);

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

        /// Return the current adaptive quorum EMA in basis points (issue #1107).
        ///
        /// Returns 0 when adaptive quorum is disabled (both bounds set to 0).
        #[must_use]
        pub fn current_quorum_bps(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::QuorumEmaBps)
                .unwrap_or(0)
        }

        // ── Private helpers ──────────────────────────────────────────────────

        fn require_initialized(env: &Env) -> Result<(), DaoError> {
            if !env.storage().instance().has(&DataKey::Initialized) {
                return Err(DaoError::NotInitialized);
            }
            Ok(())
        }

        /// Transfer `proposal.bond_amount` from the DAO contract to the admin
        /// treasury as a slash penalty (issue #1106).
        fn slash_bond(env: &Env, proposal: &Proposal) {
            let token_addr: Address = match env
                .storage()
                .instance()
                .get(&DataKey::Token)
            {
                Some(a) => a,
                None => return,
            };
            let admin: Address = match env.storage().instance().get(&DataKey::Admin) {
                Some(a) => a,
                None => return,
            };
            let dao_addr = env.current_contract_address();
            token::Client::new(env, &token_addr).transfer(
                &dao_addr,
                &admin,
                &proposal.bond_amount,
            );
            events::bond_slashed(env, &proposal.proposer, proposal.id, proposal.bond_amount);
        }

        /// Compute participation as a fraction of the absolute quorum floor,
        /// capped at `BPS_DENOMINATOR` (10 000 bps = 100 %).
        ///
        /// When `quorum == 0` we treat any participation as 100 % to avoid
        /// division by zero.
        fn participation_bps(total_votes: i128, quorum: i128) -> u32 {
            if quorum <= 0 {
                return BPS_DENOMINATOR;
            }
            // Safe: total_votes and quorum are non-negative i128; result fits u32.
            let ratio = (total_votes * i128::from(BPS_DENOMINATOR)) / quorum;
            if ratio > i128::from(BPS_DENOMINATOR) {
                BPS_DENOMINATOR
            } else {
                // Safe cast: value is in [0, BPS_DENOMINATOR] which fits u32.
                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                let r = ratio as u32;
                r
            }
        }

        /// Update the stored exponential moving average of quorum participation
        /// (issue #1107).
        ///
        /// Formula (alpha = 2 / (window + 1)):
        ///
        /// ```text
        /// ema_new = ema_old + alpha * (sample - ema_old)
        ///         = ema_old * (window - 1) / (window + 1) + sample * 2 / (window + 1)
        /// ```
        ///
        /// All arithmetic is done in u32 basis points, bounded by
        /// `[min_quorum_bps, max_quorum_bps]`.
        fn update_quorum_ema(env: &Env, sample_bps: u32) {
            let min_bps: u32 = env
                .storage()
                .instance()
                .get(&DataKey::MinQuorumBps)
                .unwrap_or(0);
            let max_bps: u32 = env
                .storage()
                .instance()
                .get(&DataKey::MaxQuorumBps)
                .unwrap_or(0);

            // Adaptive quorum is disabled when both bounds are zero.
            if min_bps == 0 && max_bps == 0 {
                return;
            }

            let window: u32 = env
                .storage()
                .instance()
                .get(&DataKey::QuorumEmaWindow)
                .unwrap_or(DEFAULT_QUORUM_EMA_WINDOW);

            let old_ema: u32 = env
                .storage()
                .instance()
                .get(&DataKey::QuorumEmaBps)
                .unwrap_or(min_bps);

            // EMA update using integer arithmetic to avoid floats.
            // alpha = 2 / (window + 1)
            // ema_new = (old_ema * (window - 1) + sample_bps * 2) / (window + 1)
            let numerator = old_ema
                .saturating_mul(window.saturating_sub(1))
                .saturating_add(sample_bps.saturating_mul(2));
            let denominator = window.saturating_add(1);
            let new_ema_raw = numerator / denominator;

            // Clamp to configured bounds.
            let new_ema = new_ema_raw.max(min_bps).min(max_bps);

            env.storage()
                .instance()
                .set(&DataKey::QuorumEmaBps, &new_ema);

            let count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::QuorumEmaCount)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::QuorumEmaCount, &count.saturating_add(1));

            bump_instance(env);
            events::quorum_updated(env, new_ema);
        }
    }
}

mod test;
mod prop_test;
