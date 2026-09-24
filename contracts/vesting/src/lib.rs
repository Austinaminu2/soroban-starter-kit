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

            // After revoke, `amount` is already capped to what was vested at revoke time.
            // We still allow claiming that remainder; once claimed == amount there's nothing left.
            let vested = if revoked {
                amount // amount was capped at revoke time
            } else {
                vested_amount(amount, cliff_ledger, end_ledger, env.ledger().sequence())
            };
            let claimable = vested - claimed;

            if claimable <= 0 {
                return Err(VestingError::NothingToClaim);
            }

            // Update the claimed amount
            schedule.claimed += claimable;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump_schedule(&env, &schedule_key);

            // Transfer the claimable amount to the beneficiary
            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &beneficiary,
                &claimable,
            );

            bump(&env);
            events::claimed(&env, &beneficiary, claimable);
            Ok(claimable)
        }

        /// Admin cancels the vesting schedule for a beneficiary. Unvested tokens are returned to admin;
        /// already-vested tokens remain claimable by the beneficiary (but no further
        /// vesting accrues after this ledger).
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::NotAuthorized`] if the caller is not the admin.
        /// - [`VestingError::AlreadyRevoked`] if already revoked.
        pub fn revoke(env: Env, beneficiary: Address) -> Result<i128, VestingError> {
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

            // Get the schedule for this beneficiary
            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            if schedule.revoked {
                return Err(VestingError::AlreadyRevoked);
            }

            let amount = schedule.amount;
            let cliff_ledger = schedule.cliff_ledger;
            let end_ledger = schedule.end_ledger;
            let claimed = schedule.claimed;

            let vested = vested_amount(amount, cliff_ledger, end_ledger, env.ledger().sequence());
            // Tokens vested but not yet claimed stay in the contract for the beneficiary.
            // Tokens not yet vested are returned to admin.
            let returnable = amount - vested;

            // Mark as revoked and cap the schedule amount to what has vested
            schedule.revoked = true;
            schedule.amount = vested;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump_schedule(&env, &schedule_key);
            // Claimed stays the same; beneficiary can still claim (vested - claimed).
            let _ = claimed; // already stored, no change needed

            if returnable > 0 {
                token::Client::new(&env, &token).transfer(
                    &env.current_contract_address(),
                    &admin,
                    &returnable,
                );
            }

            bump(&env);
            events::revoked(&env, &beneficiary, &admin, returnable);
            Ok(returnable)
        }

        /// Emergency unlock: admin releases all tokens to a beneficiary before their cliff.
        ///
        /// Only callable before the beneficiary's cliff ledger. Transfers the full unvested amount
        /// to the beneficiary, emits an `admin_released` event, and records the
        /// released amount in an on-chain audit-log entry.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::NotAuthorized`] if the caller is not the admin.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::AlreadyRevoked`] if the schedule has already been revoked.
        /// - [`VestingError::CliffAlreadyPassed`] if the cliff has already been reached.
        /// - [`VestingError::NothingToClaim`] if there are no tokens left to release.
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

            // Get the schedule for this beneficiary
            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            if schedule.revoked {
                return Err(VestingError::AlreadyRevoked);
            }

            // Only callable before the cliff.
            if env.ledger().sequence() >= schedule.cliff_ledger {
                return Err(VestingError::CliffAlreadyPassed);
            }

            let releasable = schedule.amount - schedule.claimed;
            if releasable <= 0 {
                return Err(VestingError::NothingToClaim);
            }

            // Mark as revoked, cap amount to what's being released (nothing more to claim).
            schedule.revoked = true;
            schedule.amount = releasable;
            schedule.claimed += releasable;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump_schedule(&env, &schedule_key);

            // Audit log: accumulate total admin-released tokens.
            let prev_released: i128 = env
                .storage()
                .instance()
                .get(&DataKey::AdminReleased)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::AdminReleased, &(prev_released + releasable));

            token::Client::new(&env, &token).transfer(
                &env.current_contract_address(),
                &beneficiary,
                &releasable,
            );

            bump(&env);
            events::admin_released(&env, &admin, releasable);
            Ok(releasable)
        }

        /// Returns a snapshot of the vesting schedule for a beneficiary, or `None` if not found.
        pub fn get_info(env: Env, beneficiary: Address) -> Option<BeneficiarySchedule> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return None;
            }
            bump(&env);
            let schedule_key = DataKey::Schedule(beneficiary);
            env.storage().persistent().get(&schedule_key)
        }

        /// Returns the amount claimable right now (vested minus already claimed) for a beneficiary.
        pub fn claimable(env: Env, beneficiary: Address) -> i128 {
            if !env.storage().instance().has(&DataKey::Admin) {
                return 0;
            }
            let schedule_key = DataKey::Schedule(beneficiary);
            let schedule: BeneficiarySchedule = match env.storage().persistent().get(&schedule_key) {
                Some(s) => s,
                None => return 0,
            };
            
            let amount = schedule.amount;
            let cliff_ledger = schedule.cliff_ledger;
            let end_ledger = schedule.end_ledger;
            let claimed = schedule.claimed;
            let revoked = schedule.revoked;
            // After revoke, amount is already capped to what was vested at revoke time.
            let vested = if revoked {
                amount
            } else {
                vested_amount(amount, cliff_ledger, end_ledger, env.ledger().sequence())
            };
            (vested - claimed).max(0)
        }

        /// Return the on-chain contract version number.
        pub fn contract_version(env: Env) -> u32 {
            env.storage().instance().get(&DataKey::Version).unwrap_or(0)
        }
    }
}