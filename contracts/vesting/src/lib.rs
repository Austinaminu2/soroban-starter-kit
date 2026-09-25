#![no_std]
#![deny(missing_docs)]
//! Token vesting contract template.
//!
//! An admin deposits tokens under a linear schedule with a cliff; the
//! beneficiary claims vested tokens over time and the admin may revoke the
//! unvested remainder.

#[cfg(test)]
extern crate std;

use soroban_sdk::{Address, Env, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

#[cfg(test)]
mod prop_test;
#[cfg(test)]
mod test;

pub use errors::VestingError;
pub use storage::{BeneficiarySchedule, DataKey, VestingInfo};

use soroban_common::{
    LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, extend_ttl_instance, extend_ttl_persistent,
};

fn bump(env: &Env) {
    extend_ttl_instance(env, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Extend the TTL of a beneficiary's persistent `Schedule` entry. Instance
/// storage TTL (bumped by `bump`) does not cover persistent entries — each
/// one needs its own extension or it can be archived independently of the
/// rest of the contract's state.
fn bump_schedule(env: &Env, schedule_key: &DataKey) {
    extend_ttl_persistent(env, schedule_key, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Returns the number of tokens vested as of `ledger`, ignoring already-claimed tokens.
pub(crate) fn vested_amount(amount: i128, cliff_ledger: u32, end_ledger: u32, ledger: u32) -> i128 {
    if ledger < cliff_ledger {
        return 0;
    }
    if ledger >= end_ledger {
        return amount;
    }
    // Linear interpolation between cliff and end.
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    // u32 ledger difference fits in i128
    let elapsed = (ledger - cliff_ledger) as i128;
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    // u32 ledger difference fits in i128
    let total = (end_ledger - cliff_ledger) as i128;
    amount * elapsed / total
}

fn validate_schedule(cliff_ledger: u32, end_ledger: u32, now: u32) -> Result<(), VestingError> {
    if cliff_ledger >= end_ledger || end_ledger <= now {
        return Err(VestingError::InvalidSchedule);
    }
    Ok(())
}

/// Token vesting contract with cliff + linear release schedule.
///
/// Flow:
/// 1. Admin calls `initialize` — deposits `amount` tokens and records the schedule.
/// 2. Beneficiary calls `claim` any time after the cliff to receive vested tokens.
/// 3. Admin may call `revoke` to cancel unvested tokens (returned to admin).
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct VestingContract;

    #[contractimpl]
    impl VestingContract {
        /// Initialize the vesting contract with admin and token. Must be called once before creating any schedules.
        ///
        /// # Errors
        /// - [`VestingError::AlreadyInitialized`] if called more than once.
        pub fn initialize(
            env: Env,
            admin: Address,
            token: Address,
        ) -> Result<(), VestingError> {
            if env.storage().instance().has(&DataKey::Admin) {
                return Err(VestingError::AlreadyInitialized);
            }

            admin.require_auth();

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage().instance().set(&DataKey::Token, &token);
            env.storage().instance().set(&DataKey::Version, &1u32);
            env.storage().instance().set(&DataKey::AdminReleased, &0i128);

            bump(&env);
            Ok(())
        }

        /// Create a new vesting schedule for a beneficiary and transfer `amount` tokens from the caller into the contract.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::InvalidAmount`] if `amount` <= 0.
        /// - [`VestingError::InvalidSchedule`] if `cliff_ledger` >= `end_ledger` or
        ///   `end_ledger` <= current ledger.
        /// - [`VestingError::ScheduleAlreadyExists`] if a schedule already exists for this beneficiary.
        pub fn create_schedule(
            env: Env,
            beneficiary: Address,
            cliff_ledger: u32,
            end_ledger: u32,
            amount: i128,
        ) -> Result<(), VestingError> {
            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(VestingError::NotInitialized)?;
            let token: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(VestingError::NotInitialized)?;

            admin.require_auth();

            if amount <= 0 {
                return Err(VestingError::InvalidAmount);
            }
            validate_schedule(cliff_ledger, end_ledger, env.ledger().sequence())?;

            // Check if schedule already exists for this beneficiary
            let schedule_key = DataKey::Schedule(beneficiary.clone());
            if env.storage().persistent().has(&schedule_key) {
                return Err(VestingError::ScheduleAlreadyExists);
            }

            // Store the new schedule before pulling funds from the admin. A failed transfer
            // reverts this effect atomically with the rest of the transaction.
            let schedule = BeneficiarySchedule {
                amount,
                cliff_ledger,
                end_ledger,
                claimed: 0,
                revoked: false,
            };
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);

            // Pull tokens from admin into the contract.
            token::Client::new(&env, &token).transfer(
                &admin,
                &env.current_contract_address(),
                &amount,
            );

            events::initialized(&env, &beneficiary, amount, cliff_ledger, end_ledger);
            Ok(())
        }

        /// Release all currently vested, unclaimed tokens to the beneficiary.
        ///
        /// After `revoke`, the beneficiary may still claim tokens that were vested
        /// at the time of revocation (the schedule amount is capped at that point).
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::NotAuthorized`] if caller is not the beneficiary.
        /// - [`VestingError::NothingToClaim`] if no new tokens have vested since the last claim.
        pub fn claim(env: Env, beneficiary: Address) -> Result<i128, VestingError> {
            let _admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(VestingError::NotInitialized)?;
            let token: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(VestingError::NotInitialized)?;

            // Only the beneficiary can claim their own tokens
            beneficiary.require_auth();

            // Get the schedule for this beneficiary
            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            let amount = schedule.amount;
            let cliff_ledger = schedule.cliff_ledger;
            let end_ledger = schedule.end_ledger;
            let claimed = schedule.claimed;
            let revoked = schedule.revoked;

            let vested = vested_amount(amount, cliff_ledger, end_ledger, env.ledger().sequence());
            let claimable = vested - claimed;
            if claimable <= 0 {
                return Err(VestingError::NothingToClaim);
            }

            schedule.claimed = claimed + claimable;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);

            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &beneficiary,
                &claimable,
            );

            events::claimed(&env, &beneficiary, claimable);
            let _ = revoked;
            Ok(claimable)
        }

        /// Admin emergency release: unlock all remaining unvested tokens to the
        /// beneficiary at any point in the schedule (before or after the cliff).
        ///
        /// This is intended for protocol migrations or urgent contract upgrades
        /// where the admin must move remaining tokens out of a deprecated
        /// contract without waiting for the full multi-year schedule to end.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::NotAuthorized`] if caller is not the admin.
        /// - [`VestingError::NothingToClaim`] if no unvested tokens remain.
        pub fn admin_release(env: Env, beneficiary: Address) -> Result<i128, VestingError> {
            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(VestingError::NotInitialized)?;
            let token: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(VestingError::NotInitialized)?;

            admin.require_auth();

            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            // Release all remaining unvested tokens regardless of cliff position.
            let remaining = schedule.amount - schedule.claimed;
            if remaining <= 0 {
                return Err(VestingError::NothingToClaim);
            }

            schedule.claimed = schedule.amount;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);

            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &beneficiary,
                &remaining,
            );

            events::admin_released(&env, &beneficiary, remaining);
            Ok(remaining)
        }
    }
}
