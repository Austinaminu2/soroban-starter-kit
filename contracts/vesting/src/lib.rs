#![no_std]
#![deny(missing_docs)]
//! Token vesting contract template.
//!
//! An admin deposits tokens under a linear schedule with a cliff; the
//! beneficiary claims vested tokens over time and the admin may revoke the
//! unvested remainder.

#[cfg(test)]
extern crate std;

use soroban_sdk::{Address, Env, U256, Vec, contract, contractimpl, token};

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
///
/// Uses 256-bit wide arithmetic for the `amount * elapsed` product so that
/// large token supplies (e.g. 18-decimal tokens with billions of units) and
/// long vesting durations cannot overflow. Returns
/// [`VestingError::ArithmeticError`] if the intermediate product or the final
/// downcast does not fit in `i128`.
pub(crate) fn vested_amount(
    amount: i128,
    cliff_ledger: u32,
    end_ledger: u32,
    ledger: u32,
) -> Result<i128, VestingError> {
    if ledger < cliff_ledger {
        return Ok(0);
    }
    if ledger >= end_ledger {
        return Ok(amount);
    }
    // Linear interpolation between cliff and end.
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    // u32 ledger difference fits in i128
    let elapsed = (ledger - cliff_ledger) as i128;
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
    // u32 ledger difference fits in i128
    let total = (end_ledger - cliff_ledger) as i128;

    // Widen to 256-bit unsigned arithmetic so `amount * elapsed` cannot
    // overflow for realistic supplies and durations. `amount` is validated
    // positive by `create_schedule`, so the unsigned conversion is safe.
    let amount_u = U256::from_u128(&Env::default(), amount as u128);
    let elapsed_u = U256::from_u128(&Env::default(), elapsed as u128);
    let total_u = U256::from_u128(&Env::default(), total as u128);

    let product = amount_u
        .checked_mul(&elapsed_u)
        .ok_or(VestingError::ArithmeticError)?;
    let quotient = product
        .checked_div(&total_u)
        .ok_or(VestingError::ArithmeticError)?;

    // The result is bounded by `amount` (<= i128::MAX), so the downcast is
    // exact; guard anyway to surface any unexpected overflow.
    let result = quotient
        .to_u128()
        .ok_or(VestingError::ArithmeticError)?;
    i128::try_from(result).map_err(|_| VestingError::ArithmeticError)
}

/// Returns the number of tokens vested as of `ledger` for a tranche-based
/// schedule, ignoring already-claimed tokens.
///
/// `tranches` is a list of `(ledger_sequence, percentage_bps)` pairs. Each
/// entry releases `percentage_bps` basis points (1/100th of a percent) of the
/// total `amount` once `ledger` reaches `ledger_sequence`. Percentages are
/// cumulative across tranches and must sum to at most 10_000 BPS (100%).
///
/// Uses 256-bit wide arithmetic for the `amount * bps` product so large token
/// supplies cannot overflow. Returns [`VestingError::ArithmeticError`] if the
/// intermediate product or the final downcast does not fit in `i128`.
pub(crate) fn vested_amount_tranches(
    amount: i128,
    tranches: &Vec<(u32, u32)>,
    ledger: u32,
) -> Result<i128, VestingError> {
    let mut vested: i128 = 0;
    for i in 0..tranches.len() {
        let (tranche_ledger, bps) = tranches.get(i).ok_or(VestingError::InvalidSchedule)?;
        if ledger < tranche_ledger {
            break;
        }
        // Widen to 256-bit unsigned arithmetic so `amount * bps` cannot
        // overflow for realistic supplies. `amount` is validated positive by
        // `create_schedule`, so the unsigned conversion is safe.
        let amount_u = U256::from_u128(&Env::default(), amount as u128);
        let bps_u = U256::from_u128(&Env::default(), bps as u128);
        let product = amount_u
            .checked_mul(&bps_u)
            .ok_or(VestingError::ArithmeticError)?;
        let ten_thousand = U256::from_u128(&Env::default(), 10_000u128);
        let quotient = product
            .checked_div(&ten_thousand)
            .ok_or(VestingError::ArithmeticError)?;
        let release = quotient
            .to_u128()
            .ok_or(VestingError::ArithmeticError)?;
        let release = i128::try_from(release).map_err(|_| VestingError::ArithmeticError)?;
        vested = vested
            .checked_add(release)
            .ok_or(VestingError::ArithmeticError)?;
    }
    if vested > amount {
        return Ok(amount);
    }
    Ok(vested)
}

/// Validate a tranche schedule: ledgers strictly increasing, each BPS in
/// `(0, 10_000]`, and the cumulative BPS not exceeding 10_000 (100%).
fn validate_tranches(tranches: &Vec<(u32, u32)>, now: u32) -> Result<(), VestingError> {
    if tranches.is_empty() {
        return Err(VestingError::InvalidSchedule);
    }
    let mut prev_ledger: u32 = 0;
    let mut total_bps: u32 = 0;
    for i in 0..tranches.len() {
        let (tranche_ledger, bps) = tranches.get(i).ok_or(VestingError::InvalidSchedule)?;
        if bps == 0 || bps > 10_000 {
            return Err(VestingError::InvalidSchedule);
        }
        if i > 0 && tranche_ledger <= prev_ledger {
            return Err(VestingError::InvalidSchedule);
        }
        if tranche_ledger <= now {
            return Err(VestingError::InvalidSchedule);
        }
        total_bps = total_bps
            .checked_add(bps)
            .ok_or(VestingError::InvalidSchedule)?;
        if total_bps > 10_000 {
            return Err(VestingError::InvalidSchedule);
        }
        prev_ledger = tranche_ledger;
    }
    Ok(())
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
        /// `is_revocable` controls whether the admin may later cancel the
        /// unvested remainder via [`Self::revoke`]. Pass `false` for
        /// irrevocable grants (investor agreements, team allocations) where
        /// the beneficiary requires certainty that the schedule cannot be
        /// unilaterally cancelled.
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
            is_revocable: bool,
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
                is_revocable,
                tranches: Vec::new(&env),
                milestone_oracle: None,
                milestone_released: false,
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

        /// Create a tranche-based vesting schedule for a beneficiary.
        ///
        /// `tranches` is a list of `(ledger_sequence, percentage_bps)` pairs.
        /// Each entry releases `percentage_bps` basis points of `amount` once
        /// the ledger reaches `ledger_sequence`. Ledgers must be strictly
        /// increasing, each BPS in `(0, 10_000]`, and the cumulative BPS must
        /// not exceed 10_000 (100%).
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::InvalidAmount`] if `amount` <= 0.
        /// - [`VestingError::InvalidSchedule`] if `tranches` is empty, unsorted,
        ///   contains a zero/over-100% BPS, sums above 100%, or references a
        ///   ledger at or before the current ledger.
        /// - [`VestingError::ScheduleAlreadyExists`] if a schedule already exists for this beneficiary.
        pub fn create_tranche_schedule(
            env: Env,
            beneficiary: Address,
            tranches: Vec<(u32, u32)>,
            amount: i128,
            is_revocable: bool,
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
            validate_tranches(&tranches, env.ledger().sequence())?;

            let schedule_key = DataKey::Schedule(beneficiary.clone());
            if env.storage().persistent().has(&schedule_key) {
                return Err(VestingError::ScheduleAlreadyExists);
            }

            let last_ledger = tranches
                .get(tranches.len() - 1)
                .ok_or(VestingError::InvalidSchedule)?
                .0;

            let schedule = BeneficiarySchedule {
                amount,
                cliff_ledger: last_ledger,
                end_ledger: last_ledger,
                claimed: 0,
                revoked: false,
                is_revocable,
                tranches,
                milestone_oracle: None,
                milestone_released: false,
            };
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);

            token::Client::new(&env, &token).transfer(
                &admin,
                &env.current_contract_address(),
                &amount,
            );

            events::initialized(&env, &beneficiary, amount, 0, last_ledger);
            Ok(())
        }

        /// Configure a milestone release for a beneficiary's schedule.
        ///
        /// `oracle` is the address authorized to call
        /// [`Self::release_milestone`] once the deliverable is verified. The
        /// admin may set the oracle to any address (including a multi-sig
        /// contract) at schedule creation time or later.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        pub fn set_milestone_oracle(
            env: Env,
            beneficiary: Address,
            oracle: Address,
        ) -> Result<(), VestingError> {
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

            schedule.milestone_oracle = Some(oracle);
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);
            Ok(())
        }

        /// Release the milestone portion of a beneficiary's schedule.
        ///
        /// Must be called by the configured milestone oracle (which may be a
        /// multi-sig contract). Once released, the full `amount` becomes
        /// vested and claimable by the beneficiary.
        ///
        /// # Errors
        /// - [`VestingError::NotInitialized`] if the contract has not been initialized.
        /// - [`VestingError::ScheduleNotFound`] if no schedule exists for the beneficiary.
        /// - [`VestingError::Unauthorized`] if the caller is not the configured oracle.
        /// - [`VestingError::MilestoneAlreadyReleased`] if the milestone was already released.
        pub fn release_milestone(
            env: Env,
            beneficiary: Address,
        ) -> Result<(), VestingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(VestingError::NotInitialized);
            }

            let schedule_key = DataKey::Schedule(beneficiary.clone());
            let mut schedule: BeneficiarySchedule = env
                .storage()
                .persistent()
                .get(&schedule_key)
                .ok_or(VestingError::ScheduleNotFound)?;

            let oracle = schedule
                .milestone_oracle
                .clone()
                .ok_or(VestingError::Unauthorized)?;
            oracle.require_auth();

            if schedule.milestone_released {
                return Err(VestingError::MilestoneAlreadyReleased);
            }

            schedule.milestone_released = true;
            env.storage().persistent().set(&schedule_key, &schedule);
            bump(&env);
            bump_schedule(&env, &schedule_key);
            Ok(())
        }

        /// Reassign a vesting schedule from `current_beneficiary` to
    

/* … truncated 2001 chars — edit only what you need near the top … */
