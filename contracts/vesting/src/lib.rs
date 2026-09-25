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
///
/// Revocation may be configured with a grace period (`revocation_delay` ledgers)
/// during which vesting continues and the beneficiary can still claim. The
/// revocation is only finalized once the delay has elapsed.
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
            // Default: no grace period (immediate revocation) unless configured.
            env.storage().instance().set(&DataKey::RevocationDelay, &0u32);

            bump(&env);
            Ok(())
        }

        /// Configure the revocation grace period, in ledgers. Only the admin may
        /// call this. A value of `0` restores immediate revocation.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        pub fn set_revocation_delay(env: Env, delay: u32) -> Result<(), VestingError> {
            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(VestingError::NotInitialized)?;
            admin.require_auth();

            env.storage().instance().set(&DataKey::RevocationDelay, &delay);
            bump(&env);
            Ok(())
        }

        /// Return the currently configured revocation grace period, in ledgers.
        pub fn revocation_delay(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::RevocationDelay)
                .unwrap_or(0)
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
                revocation_pending: false,
                revocation_ledger: 0,
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
        /// While a revocation is pending, vesting continues and the beneficiary may
        /// claim tokens that vest during the grace period.
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

            let now = env.ledger().sequence();

            // Determine the effective end of vesting. If the schedule has been
            // revoked, vesting stops at the revocation ledger. If a revocation is
            // pending, vesting continues until the grace period elapses, at which
            // point it is capped at the finalization ledger.
            let effective_end = if revoked {
                schedule.revocation_ledger
            } else if schedule.revocation_pending {
                let finalize_ledger = schedule
                    .revocation_ledger
                    .saturating_add(revocation_delay(&env));
                if now >= finalize_ledger {
                    finalize_ledger
                } else {
                    end_ledger
                }
            } else {
                end_ledger
            };

            let vested = vested_amount(amount, cliff_ledger, effective_end, now);
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
            Ok(claimable)
        }

        /// Revoke a beneficiary's schedule. If a revocation grace period is
        /// configured, the schedule transitions to a pending state and vesting
        /// continues until the delay elapses; otherwise the revocation is applied
        /// immediately.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::AlreadyRevoked`] if the schedule is already revoked or pending.
        pub fn revoke(env: Env, beneficiary: Address) -> Result<(), VestingError> {
            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(VestingError::NotInitialized)?;
            admin.require_auth();

            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            if schedule.revoked || schedule.revocation_pending {
                return Err(VestingError::AlreadyRevoked);
            }

            let now = env.ledger().sequence();
            let delay = revocation_delay(&env);

            if delay == 0 {
                schedule.revoked = true;
                schedule.revocation_ledger = now;
            } else {
                schedule.revocation_pending = true;
                schedule.revocation_ledger = now;
            }

            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);

            events::revoked(&env, &beneficiary, now);
            Ok(())
        }

        /// Finalize a pending revocation once the grace period has elapsed. The
        /// unvested remainder is returned to the admin and the schedule is marked
        /// revoked. Callable by anyone; it is a no-op error if not yet due.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::AlreadyRevoked`] if the schedule is not pending revocation.
        /// - [`VestingError::RevocationNotDue`] if the grace period has not elapsed.
        pub fn finalize_revocation(env: Env, beneficiary: Address) -> Result<(), VestingError> {
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

            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            if schedule.revoked || !schedule.revocation_pending {
                return Err(VestingError::AlreadyRevoked);
            }

            let now = env.ledger().sequence();
            let finalize_ledger = schedule
                .revocation_ledger
                .saturating_add(revocation_delay(&env));
            if now < finalize_ledger {
                return Err(VestingError::RevocationNotDue);
            }

            // Cap vesting at the finalization ledger and return the unvested
            // remainder to the admin.
            let vested = vested_amount(
                schedule.amount,
                schedule.cliff_ledger,
                finalize_ledger,
                now,
            );
            let unvested = schedule.amount - vested;

            schedule.revoked = true;
            schedule.revocation_pending = false;
            schedule.revocation_ledger = finalize_ledger;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);

            if unvested > 0 {
                let admin: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::Admin)
                    .ok_or(VestingError::NotInitialized)?;
                token::Client::new(&env, &token).transfer(
                    &env.current_contract_address(),
                    &admin,
                    &unvested,
                );
            }

            events::revoked(&env, &beneficiary, finalize_ledger);
            Ok(())
        }
    }
}
