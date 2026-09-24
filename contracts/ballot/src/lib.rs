#![no_std]
#![deny(missing_docs)]
//! Multi-choice on-chain ballot contract template.
//!
//! Voters cast a single vote among N registered choices; results are tallied
//! on-chain once voting closes.
//!
//! ## Multi-choice ballot (#788)
//!
//! `initialize` now accepts a `choices: Vec<String>` parameter — an ordered
//! list of named options (e.g. `["yes", "no", "abstain"]`).  The `choice`
//! argument to `vote` is an index into that list (0-based).  Two read helpers
//! are provided:
//!
//! - `tally_all()` — returns `Vec<i128>` of per-choice vote counts in
//!   declaration order, then closes voting.
//! - `tally()` — backward-compatible two-choice helper that returns
//!   `(choice[1] votes, choice[0] votes)` i.e. `(yes, no)`.
//!
//! Contracts with more than two choices should call `tally_all()`.
//!
//! ## Voting window (#787)
//!
//! `initialize` accepts `voting_start` and `voting_end` ledger sequences.
//! `vote()` is rejected with [`BallotError::VotingNotStarted`] before the
//! window opens and with [`BallotError::VotingClosed`] after it closes.
//!
//! ## Voter deregistration (#786)
//!
//! `deregister_voter` (admin-only) removes a registered voter, but only while
//! no vote has yet been cast.
//!
//! ## Batch voter registration (#1122)
//!
//! `register_voters_batch` (admin-only) registers many voters in a single
//! invocation, skipping any that are already registered.

use soroban_sdk::{Address, Env, String, Vec, contract, contractimpl};

mod errors;
mod events;
mod storage;

pub use errors::BallotError;
pub use storage::DataKey;

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, extend_ttl_instance};

fn bump(env: &Env) {
    extend_ttl_instance(env, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Multi-choice on-chain ballot contract.
///
/// Flow:
/// 1. Admin calls `initialize` — sets up N named choices and a voting window.
/// 2. Admin calls `register_voter` to add voters.  Mistakes may be undone with
///    `deregister_voter` before any vote is cast.
/// 3. Voters call `vote(voter, choice_index)` within the voting window.
/// 4. Admin calls `tally_all()` (or `tally()` for two-choice ballots) to get
///    final results and close voting.
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct BallotContract;

    #[contractimpl]
    impl BallotContract {
        /// Initialize the ballot contract with N named choices.
        ///
        /// `voting_start` and `voting_end` are inclusive ledger sequence numbers
        /// defining the window during which votes are accepted.
        ///
        /// `choices` must be non-empty.  The index of each element becomes the
        /// `choice` value accepted by `vote`.
        ///
        /// # Errors
        /// - [`BallotError::AlreadyInitialized`] if called more than once.
        /// - [`BallotError::NoChoices`] if `choices` is empty.
        /// - [`BallotError::InvalidWindow`] if `voting_start >= voting_end` or
        ///   `voting_end <= current ledger`.
        pub fn initialize(
            env: Env,
            admin: Address,
            voting_start: u32,
            voting_end: u32,
            choices: Vec<String>,
        ) -> Result<(), BallotError> {
            if env.storage().instance().has(&DataKey::Admin) {
                return Err(BallotError::AlreadyInitialized);
            }
            if choices.is_empty() {
                return Err(BallotError::NoChoices);
            }
            if voting_start >= voting_end || voting_end <= env.ledger().sequence() {
                return Err(BallotError::InvalidWindow);
            }
            admin.require_auth();

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage().instance().set(&DataKey::VotingActive, &true);
            env.storage()
                .instance()
                .set(&DataKey::VotingStart, &voting_start);
            env.storage()
                .instance()
                .set(&DataKey::VotingEnd, &voting_end);
            env.storage().instance().set(&DataKey::TotalVotes, &0i128);

            // Store the choices list.
            let choice_count = choices.len();
            env.storage().instance().set(&DataKey::Choices, &choices);

            // Initialise per-choice counters to zero.
            for i in 0..choice_count {
                env.storage()
                    .instance()
                    .set(&DataKey::ChoiceVotes(i), &0i128);
            }

            // Backward-compat counters: slot 0 → NoVotes, slot 1 → YesVotes.
            env.storage().instance().set(&DataKey::YesVotes, &0i128);
            env.storage().instance().set(&DataKey::NoVotes, &0i128);

            bump(&env);
            events::initialized(&env, &admin);
            Ok(())
        }

        /// Admin registers a voter for the ballot.
        ///
        /// # Errors
        /// - [`BallotError::NotInitialized`] if the contract has not been initialized.
        /// - [`BallotError::Unauthorized`] if the caller is not the admin.
        pub fn register_voter(env: Env, voter: Address) -> Result<(), BallotError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(BallotError::NotInitialized);
            }

            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(BallotError::NotInitialized)?;
            admin.require_auth();

            let voter_key = DataKey::RegisteredVoter(voter.clone());
            env.storage().persistent().set(&voter_key, &true);
            env.storage().persistent().extend_ttl(
                &voter_key,
                LEDGER_LIFETIME_THRESHOLD,
                LEDGER_BUMP_AMOUNT,
            );

            bump(&env);
            events::voter_registered(&env, &voter);
            Ok(())
        }

        /// Admin registers many voters in a single invocation.
        ///
        /// Voters already registered (or duplicated within `voters`) are
        /// skipped, so the call is idempotent.  Storage TTL is extended for
        /// every newly registered voter.
        ///
        /// # Errors
        /// - [`BallotError::NotInitialized`] if the contract has not been initialized.
        /// - [`BallotError::Unauthorized`] if the caller is not the admin.
        pub fn register_voters_batch(
            env: Env,
            voters: Vec<Address>,
        ) -> Result<(), BallotError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(BallotError::NotInitialized);
            }

            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(BallotError::NotInitialized)?;
            admin.require_auth();

            let mut registered: u32 = 0;
            for voter in voters.iter() {
                let voter_key = DataKey::RegisteredVoter(voter.clone());
                let already: bool = env
                    .storage()
                    .persistent()
                    .get(&voter_key)
                    .unwrap_or(false);
                if already {
                    continue;
                }

                env.storage().persistent().set(&voter_key, &true);
                env.storage().persistent().extend_ttl(
                    &voter_key,
                    LEDGER_LIFETIME_THRESHOLD,
                    LEDGER_BUMP_AMOUNT,
                );
                registered += 1;
            }

            bump(&env);
            events::voters_registered_batch(&env, registered);
            Ok(())
        }

        /// Admin deregisters a voter, correcting a registration mistake.
        ///
        /// Only allowed before any vote has been cast.
        ///
        /// # Errors
        /// - [`BallotError::NotInitialized`]
        /// - [`BallotError::Unauthorized`]
        /// - [`BallotError::VotingAlreadyStarted`] if at least one vote has been cast.
        /// - [`BallotError::NotRegistered`] if the voter is not currently registered.
        pub fn deregister_voter(env: Env, voter: Address) -> Result<(), BallotError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(BallotError::NotInitialized);
            }

            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(BallotError::NotInitialized)?;
            admin.require_auth();

            // Reject once any vote has been cast.
            let total_votes: i128 = env
                .storage()
                .instance()
                .get(&DataKey::TotalVotes)
                .unwrap_or(0i128);
            if total_votes > 0 {
                return Err(BallotError::VotingAlreadyStarted);
            }

            let is_registered: bool = env
                .storage()
                .persistent()
                .get(&DataKey::RegisteredVoter(voter.clone()))
                .unwrap_or(false);
            if !is_registered {
                return Err(BallotError::NotRegistered);
            }

            env.storage()
                .persistent()
                .remove(&DataKey::RegisteredVoter(voter.clone()));

            bump(&env);
            events::voter_deregistered(&env, &voter);
            Ok(())
        }

        /// Voter casts their vote.
        ///
        /// `choice` is a 0-based index into the `choices` list supplied at
        /// `initialize`.  For two-choice ballots the conventional mapping is
        /// `0 = no`, `1 = yes`.
        ///
    

/* … truncated 9012 chars — edit only what you need near the top … */
