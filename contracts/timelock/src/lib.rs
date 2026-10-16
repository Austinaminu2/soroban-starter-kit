#![no_std]
#![deny(missing_docs)]
//! Timelock contract template.
//!
//! Locks tokens until a target ledger, after which the beneficiary can release
//! them; the depositor may cancel while still active.

use soroban_sdk::{Address, Env, Vec, contract, contractimpl, token};

/// Number of tranches stored in persistent storage.
const TRANCHE_COUNT: u32 = 0;

mod errors;
mod events;
mod storage;

pub use errors::TimelockError;
pub use storage::{DataKey, ReleaseTranche, TimelockInfo, TimelockState};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD};

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn get_required<T: soroban_sdk::TryFromVal<soroban_sdk::Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<T, TimelockError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(TimelockError::NotInitialized)
}

/// Timelock contract: holds tokens until a specified ledger, then releases to a beneficiary.
///
/// Lifecycle: `Active → Released` (via `release`) or `Active → Cancelled` (via `cancel`).
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;
    use storage::DataKey::*;

    #[contract]
    pub struct TimelockContract;

    #[contractimpl]
    impl TimelockContract {
        /// Initialize the timelock. Transfers `amount` tokens from `admin` to the contract.
        ///
        /// # Errors
        ///
        /// Returns [`TimelockError::AlreadyInitialized`] if already set up.
        /// Returns [`TimelockError::InvalidAmount`] if `amount` <= 0.
        /// Returns [`TimelockError::InvalidReleaseLedger`] if `release_ledger` <= current ledger.
        pub fn initialize(
            env: Env,
            admin: Address,
            token: Address,
            beneficiary: Address,
            release_ledger: u32,
            amount: i128,
        ) -> Result<(), TimelockError> {
            if env.storage().instance().has(&State) {
                return Err(TimelockError::AlreadyInitialized);
            }
            if amount <= 0 {
                return Err(TimelockError::InvalidAmount);
            }
            if release_ledger <= env.ledger().sequence() {
                return Err(TimelockError::InvalidReleaseLedger);
            }

            admin.require_auth();

            token::Client::new(&env, &token).transfer(
                &admin,
                &env.current_contract_address(),
                &amount,
            );

            env.storage().instance().set(&Admin, &admin);
            env.storage().instance().set(&Token, &token);
            env.storage().instance().set(&Beneficiary, &beneficiary);
            env.storage()
                .instance()
                .set(&ReleaseLedger, &release_ledger);
            env.storage().instance().set(&Amount, &amount);
            env.storage().instance().set(&State, &TimelockState::Active);

            bump_instance(&env);
            events::initialized(&env, &admin, &beneficiary, release_ledger, amount);

            Ok(())
        }

        /// Initialize the timelock with multiple release tranches.
        /// Transfers total tokens from `admin` to the contract.
        ///
        /// # Errors
        ///
        /// Returns [`TimelockError::AlreadyInitialized`] if already set up.
        /// Returns [`TimelockError::InvalidAmount`] if any tranche amount <= 0.
        /// Returns [`TimelockError::InvalidReleaseLedger`] if first tranche ledger <= current ledger.
        pub fn initialize_with_tranches(
            env: Env,
            admin: Address,
            token: Address,
            beneficiary: Address,
            tranches: Vec<ReleaseTranche>,
        ) -> Result<(), TimelockError> {
            if env.storage().instance().has(&State) {
                return Err(TimelockError::AlreadyInitialized);
            }
            if tranches.is_empty() {
                return Err(TimelockError::InvalidAmount);
            }

            admin.require_auth();

            // Validate tranches and calculate total amount
            let mut total_amount: i128 = 0;
            let mut prev_ledger: u32 = 0;
            for tranche in tranches.iter() {
                if tranche.amount <= 0 {
                    return Err(TimelockError::InvalidAmount);
                }
                if tranche.release_ledger <= prev_ledger {
                    return Err(TimelockError::InvalidReleaseLedger);
                }
                total_amount = total_amount.checked_add(tranche.amount)
                    .ok_or(TimelockError::InvalidAmount)?;
                prev_ledger = tranche.release_ledger;
            }

            if prev_ledger <= env.ledger().sequence() {
                return Err(TimelockError::InvalidReleaseLedger);
            }

            token::Client::new(&env, &token).transfer(
                &admin,
                &env.current_contract_address(),
                &total_amount,
            );

            env.storage().instance().set(&Admin, &admin);
            env.storage().instance().set(&Token, &token);
            env.storage().instance().set(&Beneficiary, &beneficiary);

            // Store each tranche and its release flag in persistent storage.
            let released_count = tranches.len();
            for i in 0..released_count {
                let tranche = tranches.get(i).unwrap();
                let tranche_key = DataKey::Tranche(i);
                env.storage().persistent().set(&tranche_key, &tranche);
                env.storage().persistent().extend_ttl(
                    &tranche_key,
                    LEDGER_LIFETIME_THRESHOLD,
                    LEDGER_BUMP_AMOUNT,
                );

                let released_key = DataKey::TrancheReleased(i);
                env.storage().persistent().set(&released_key, &false);
                env.storage().persistent().extend_ttl(
                    &released_key,
                    LEDGER_LIFETIME_THRESHOLD,
                    LEDGER_BUMP_AMOUNT,
                );
            }
            env.storage().instance().set(&TRANCHE_COUNT, &released_count);

            // Also set legacy fields for backwards compatibility
            let first_tranche = tranches.get(0).unwrap();
            env.storage().instance().set(&ReleaseLedger, &first_tranche.release_ledger);
            env.storage().instance().set(&Amount, &total_amount);

            env.storage().instance().set(&State, &TimelockState::Active);

            bump_instance(&env);
            events::initialized(&env, &admin, &beneficiary, first_tranche.release_ledger, total_amount);

            Ok(())
        }

        /// Release locked tokens to the beneficiary. Callable by anyone after `release_ledger`.
        ///
        /// # Errors
        ///
        /// Returns [`TimelockError::NotInitialized`] if not yet set up.
        /// Returns [`TimelockError::AlreadyReleased`] if tokens were already released.
        /// Returns [`TimelockError::AlreadyCancelled`] if the timelock was cancelled.
        /// Returns [`TimelockError::NotYetReleasable`] if `release_ledger` has not been reached.
        pub fn release(env: Env) -> Result<(), TimelockError> {
            let state: TimelockState = get_required(&env, &State)?;
            match state {
                TimelockState::Released => return Err(TimelockError::AlreadyReleased),
                TimelockState::Cancelled => return Err(TimelockError::AlreadyCancelled),
                TimelockState::Active => {}
            }

            let token: Address = get_required(&env, &Token)?;
            let beneficiary: Address = get_required(&env, &Beneficiary)?;

            // Check if this is a multi-tranche timelock
            let tranche_count: Option<u32> = env.storage().instance().get(&TRANCHE_COUNT);

            if let Some(count) = tranche_count {
                // Multi-tranche release
                let current_ledger = env.ledger().sequence();
                let mut total_released: i128 = 0;
                let mut any_new_released = false;
                let mut all_released = true;

                for i in 0..count {
                    let tranche_key = DataKey::Tranche(i);
                    let tranche: ReleaseTranche = env
                        .storage()
                        .persistent()
                        .get(&tranche_key)
                        .ok_or(TimelockError::NotInitialized)?;
                    env.storage().persistent().extend_ttl(
                        &tranche_key,
                        LEDGER_LIFETIME_THRESHOLD,
                        LEDGER_BUMP_AMOUNT,
                    );

                    let released_key = DataKey::TrancheReleased(i);
                    let is_released: bool = env
                        .storage()
                        .persistent()
                        .get(&released_key)
                        .unwrap_or(false);
                    env.storage().persistent().extend_ttl(
                        &released_key,
                        LEDGER_LIFETIME_THRESHOLD,
                        LEDGER_BUMP_AMOUNT,
                    );

                    if !is_released && current_ledger >= tranche.release_ledger {
                        env.storage().persistent().set(&released_key, &true);
                        env.storage().persistent().extend_ttl(
                            &released_key,
                            LEDGER_LIFETIME_THRESHOLD,
                            LEDGER_BUMP_AMOUNT,
                        );
                        total_released += tranche.amount;
                        any_new_released = true;
                    } else if !is_released {
                        all_released = false;
                    }
                }

                if !any_new_released {
                    return Err(TimelockError::NotYetReleasable);
                }

                if all_released {
                    env.storage().instance().set(&State, &TimelockState::Released);
                }
                bump_instance(&env);

                token::Client::new(&env, &token).transfer(
                    &env.current_contract_address(),
                    &beneficiary,
                    &total_released,
                );

                events::released(&env, &beneficiary, total_released);
            } else {
                // Legacy single-tranche release
                let release_ledger: u32 = get_required(&env, &ReleaseLedger)?;
                if env.ledger().sequence() < release_ledger {
                    return Err(TimelockError::NotYetReleasable);
                }

                let amount: i128 = get_required(&env, &Amount)?;

                env.storage()
                    .instance()
                    .set(&State, &TimelockState::Released);
                bump_instance(&env);

                token::Client::new(&env, &token).transfer(
                    &env.current_contract_address(),
                    &beneficiary,
                    &amount,
                );

                events::released(&env, &beneficiary, amount);
            }

            Ok(())
        }

        /// Cancel the timelock and return tokens to admin. Admin only; works while in `Active` state.
        ///
        /// # Errors
        ///
        /// Returns [`TimelockError::NotInitialized`] if not yet set up.
        /// Returns [`TimelockError::NotAuthorized`] if caller is not the admin.
        /// Returns [`TimelockError::AlreadyReleased`] if tokens were already released.
        /// Returns [`TimelockError::AlreadyCancelled`] if already cancelled.
        pub fn cancel(env: Env) -> Result<(), TimelockError> {
            let admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();

            let state: TimelockState = get_required(&env, &State)?;
            match state {
                TimelockState::Released => return Err(TimelockError::AlreadyReleased),
                TimelockState::Cancelled => return Err(TimelockError::AlreadyCancelled),
                TimelockState::Active => {}
            }

            let token: Address = get_required(&env, &Token)?;
            let amount: i128 = get_required(&env, &Amount)?;

            env.storage()
                .instance()
                .set(&State, &TimelockState::Cancelled);
            bump_instance(&env);

            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &admin,
                &amount,
            );

            events::cancelled(&env, &admin, amount);

            Ok(())
        }

        /// Admin-only: reassign the beneficiary while the timelock is active.
        ///
        /// # Errors
        /// - [`TimelockError::NotInitialized`] if the contract has not been initialized.
        /// - [`TimelockError::NotAuthorized`] if the caller is not the admin.
        /// - [`TimelockError::AlreadyReleased`] if the timelock has been released.
        /// - [`TimelockError::AlreadyCancelled`] if the timelock has been cancelled.
        pub fn reassign_beneficiary(
            env: Env,
            new_beneficiary: Address,
        ) -> Result<(), TimelockError> {
            let admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();

            let state: TimelockState = get_required(&env, &State)?;
            match state {
                TimelockState::Released => return Err(TimelockError::AlreadyReleased),
                TimelockState::Cancelled => return Err(TimelockError::AlreadyCancelled),
                TimelockState::Active => {}
            }

            let old_beneficiary: Address = get_required(&env, &Beneficiary)?;
            env.storage().instance().set(&Beneficiary, &new_beneficiary);
            bump_instance(&env);

            events::beneficiary_reassigned(&env, &admin, &old_beneficiary, &new_beneficiary);

            Ok(())
        }

        /// Return full timelock details.
        #[must_use]
        pub fn get_info(env: Env) -> Result<TimelockInfo, TimelockError> {
            let mut tranches: Vec<ReleaseTranche> = Vec::new(&env);
            if let Some(count) = env.storage().instance().get::<_, u32>(&TRANCHE_COUNT) {
                for i in 0..count {
                    let tranche_key = DataKey::Tranche(i);
                    if let Some(tranche) = env.storage().persistent().get(&tranche_key) {
                        env.storage().persistent().extend_ttl(
                            &tranche_key,
                            LEDGER_LIFETIME_THRESHOLD,
                            LEDGER_BUMP_AMOUNT,
                        );
                        tranches.push_back(tranche);
                    }
                }
            }

            Ok(TimelockInfo {
                admin: get_required(&env, &Admin)?,
                token: get_required(&env, &Token)?,
                beneficiary: get_required(&env, &Beneficiary)?,
                release_ledger: get_required(&env, &ReleaseLedger)?,
                amount: get_required(&env, &Amount)?,
                state: get_required(&env, &State)?,
                tranches,
            })
        }

        /// Return `true` if at least one tranche is releasable and the state is still `Active`.
        #[must_use]
        pub fn is_releasable(env: Env) -> bool {
            let state: Option<TimelockState> = env.storage().instance().get(&State);
            if !matches!(state, Some(TimelockState::Active)) {
                return false;
            }

            let current_ledger = env.ledger().sequence();

            // Check for multi-tranche timelock
            if let Some(count) = env.storage().instance().get::<_, u32>(&TRANCHE_COUNT) {
                for i in 0..count {
                    let tranche_key = DataKey::Tranche(i);
                    if let Some(tranche) = env
                        .storage()
                        .persistent()
                        .get::<_, ReleaseTranche>(&tranche_key)
                    {
                        env.storage().persistent().extend_ttl(
                            &tranche_key,
                            LEDGER_LIFETIME_THRESHOLD,
                            LEDGER_BUMP_AMOUNT,
                        );
                        if current_ledger >= tranche.release_ledger {
                            return true;
                        }
                    }
                }
                return false;
            }

            // Legacy single-tranche check
            let release_ledger: u32 = env.storage().instance().get(&ReleaseLedger).unwrap_or(0);
            current_ledger >= release_ledger
        }

        /// Return ledgers remaining until release (negative if already past).
        pub fn get_remaining_ledgers(env: Env) -> i64 {
            let release_ledger: u32 = env.storage().instance().get(&ReleaseLedger).unwrap_or(0);
            release_ledger as i64 - env.ledger().sequence() as i64
        }
    }
}

mod test;

#[cfg(test)]
mod prop_test;
