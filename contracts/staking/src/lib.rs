#![no_std]
#![deny(missing_docs)]
//! Staking rewards contract template.
//!
//! Users stake tokens to earn rewards that accrue over time from a reward pool
//! funded by the admin; rewards can be claimed independently of withdrawals.
//!
//! ## Unbonding period (#827)
//!
//! When `unbonding_period > 0` (set at initialization), calling `unstake` no
//! longer immediately returns the tokens.  Instead it records an
//! [`UnbondRequest`] and only the subsequent `withdraw` call — made after
//! `unbonding_period` ledgers have elapsed — actually transfers the tokens
//! back.  Set `unbonding_period = 0` to restore the original instant-withdraw
//! behaviour.
//!
//! ## Admin slashing (#828)
//!
//! `slash(staker, amount)` is an admin-only entry point that reduces a
//! staker's balance by up to their full current stake.  The slashed tokens are
//! routed to the `slash_destination` address supplied at initialization (this
//! can be a burn address or a treasury contract).
//!
//! ## Undistributed rewards (#1129)
//!
//! When `add_rewards` is called while `total_staked == 0`, the deposited
//! tokens are held in [`DataKey::UndistributedRewards`] instead of being
//! silently trapped.  They are folded into the global reward-per-token
//! accumulator the next time rewards are added with a non-zero stake, or when
//! the first staker joins.  If the pool stays empty the admin can reclaim them
//! via `reclaim_undistributed_rewards`.

use soroban_sdk::{Address, Env, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

#[cfg(test)]
mod test;

#[cfg(test)]
mod prop_test;

pub use errors::StakingError;
pub use storage::{DataKey, REWARD_SCALE, UnbondRequest};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, extend_ttl_instance};

fn bump(env: &Env) {
    extend_ttl_instance(env, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Returns the current global reward-per-token accumulator.
fn reward_per_token(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::RewardPerTokenStored)
        .unwrap_or(0i128)
}

/// Returns the amount of rewards deposited while no tokens were staked.
fn undistributed_rewards(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::UndistributedRewards)
        .unwrap_or(0i128)
}

/// Folds any held undistributed rewards into the reward-per-token accumulator.
///
/// Must only be called when `total_staked > 0`; otherwise the rewards would be
/// divided by zero.  Returns the amount that was folded in.
fn distribute_undistributed(env: &Env, total_staked: i128) -> i128 {
    let held = undistributed_rewards(env);
    if held <= 0 || total_staked <= 0 {
        return 0;
    }
    let rpt = reward_per_token(env);
    let new_rpt = rpt + held * REWARD_SCALE / total_staked;
    env.storage()
        .instance()
        .set(&DataKey::RewardPerTokenStored, &new_rpt);
    env.storage()
        .instance()
        .set(&DataKey::UndistributedRewards, &0i128);
    held
}

/// Helper to get admin address or return NotInitialized error.
fn get_admin(env: &Env) -> Result<Address, StakingError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(StakingError::NotInitialized)
}

/// Helper to get stake token address or return NotInitialized error.
fn get_stake_token(env: &Env) -> Result<Address, StakingError> {
    env.storage()
        .instance()
        .get(&DataKey::StakeToken)
        .ok_or(StakingError::NotInitialized)
}

/// Helper to get reward token address or return NotInitialized error.
fn get_reward_token(env: &Env) -> Result<Address, StakingError> {
    env.storage()
        .instance()
        .get(&DataKey::RewardToken)
        .ok_or(StakingError::NotInitialized)
}

/// Helper to get total staked or return NotInitialized error.
fn get_total_staked_internal(env: &Env) -> Result<i128, StakingError> {
    env.storage()
        .instance()
        .get(&DataKey::TotalStaked)
        .ok_or(StakingError::NotInitialized)
}

/// Helper to get total rewards or return NotInitialized error.
fn get_total_rewards_internal(env: &Env) -> Result<i128, StakingError> {
    env.storage()
        .instance()
        .get(&DataKey::TotalRewards)
        .ok_or(StakingError::NotInitialized)
}

/// Pure reward calculation — isolated from storage for testability.
#[allow(clippy::arithmetic_side_effects)] // overflow checked via REWARD_SCALE invariant
pub(crate) fn calculate_earned(stake: i128, rpt: i128, paid: i128, accrued: i128) -> i128 {
    accrued + stake * (rpt - paid) / REWARD_SCALE
}

/// Computes how many reward tokens `staker` has earned since their last update.
fn earned(env: &Env, staker: &Address) -> i128 {
    let stake: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Stake(staker.clone()))
        .unwrap_or(0i128);
    let rpt = reward_per_token(env);
    let paid: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::RewardPerTokenPaid(staker.clone()))
        .unwrap_or(0i128);
    let accrued: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::Rewards(staker.clone()))
        .unwrap_or(0i128);
    calculate_earned(stake, rpt, paid, accrued)
}

/// Snapshots the staker's earned rewards and updates their paid-up-to pointer.
fn update_reward(env: &Env, staker: &Address) {
    let e = earned(env, staker);
    let rpt = reward_per_token(env);
    env.storage()
        .persistent()
        .set(&DataKey::Rewards(staker.clone()), &e);
    env.storage()
        .persistent()
        .set(&DataKey::RewardPerTokenPaid(staker.clone()), &rpt);
}

/// Simple proportional token staking contract.
///
/// Flow:
/// 1. Admin calls `initialize` — sets the stake and reward token addresses,
///    unbonding period, and slash destination.
/// 2. Admin calls `add_rewards` to deposit reward tokens into the pool.
///    The global reward-per-token accumulator is updated proportionally.
/// 3. Users call `stake` to deposit stake tokens.
/// 4. Users call `claim_rewards` to collect accrued rewards.
/// 5. Users call `unstake` to queue a withdrawal (starts unbonding timer).
/// 6. After the unbonding period, users call `withdraw` to receive tokens.
///    If `unbonding_period == 0`, `unstake` transfers tokens immediately
///    (legacy behaviour).
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct StakingContract;

    #[contractimpl]
    impl StakingContract {
        /// Initialize the staking contract.
        ///
        /// - `unbonding_period` — ledgers between `unstake` and `withdraw`.
        ///   Pass `0` for immediate withdrawals (legacy behaviour).
        /// - `slash_destination` — address that receives slashed tokens.
        ///
        /// # Errors
        /// - [`StakingError::AlreadyInitialized`] if called more than once.
        pub fn initialize(
            env: Env,
            admin: Address,
            stake_token: Address,
            reward_token: Address,
            unbonding_period: u32,
            slash_destination: Address,
        ) -> Result<(), StakingError> {
            if env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::AlreadyInitialized);
            }
            admin.require_auth();

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage()
                .instance()
                .set(&DataKey::StakeToken, &stake_token);
            env.storage()
                .instance()
                .set(&DataKey::RewardToken, &reward_token);
            env.storage().instance().set(&DataKey::TotalStaked, &0i128);
            env.storage().instance().set(&DataKey::TotalRewards, &0i128);
            env.storage()
                .instance()
                .set(&DataKey::RewardPerTokenStored, &0i128);
            env.storage()
                .instance()
                .set(&DataKey::UndistributedRewards, &0i128);
            env.storage()
                .instance()
                .set(&DataKey::UnbondingPeriod, &unbonding_period);
            env.storage()
                .instance()
                .set(&DataKey::SlashDestination, &slash_destination);
            env.storage().instance().set(&DataKey::Version, &1u32);

            bump(&env);
            events::initialized(&env, &admin, &stake_token, &reward_token);
            Ok(())
        }

        /// Deposit `amount` stake tokens from `staker` into the contract.
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::InvalidAmount`] if `amount` <= 0.
        pub fn stake(env: Env, staker: Address, amount: i128) -> Result<(), StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            if amount <= 0 {
                return Err(StakingError::InvalidAmount);
            }
            staker.require_auth();

            update_reward(&env, &staker);

            let stake_token = get_stake_token(&env)?;
            token::Client::new(&env, &stake_token).transfer(
                &staker,
                &env.current_contract_address(),
                &amount,
            );

            let prev: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Stake(staker.clone()))
                .unwrap_or(0i128);
            env.storage()
                .persistent()
                .set(&DataKey::Stake(staker.clone()), &(prev + amount));

            let total_staked = get_total_staked_internal(&env)?;
            let new_total = total_staked + amount;
            env.storage()
                .instance()
                .set(&DataKey::TotalStaked, &new_total);

            // Fold any rewards that were deposited while the pool was empty so
            // the first staker (and everyone after) can earn them (#1129).
            distribute_undistributed(&env, new_total);

            bump(&env);
            events::staked(&env, &staker, amount);
            Ok(())
        }

        /// Deposit `amount` reward tokens from the admin into the pool.
        ///
        /// If no tokens are currently staked the rewards are held in
        /// [`DataKey::UndistributedRewards`] and distributed when stakers join.
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::InvalidAmount`] if `amount` <= 0.
        pub fn add_rewards(env: Env, admin: Address, amount: i128) -> Result<(), StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            if amount <= 0 {
                return Err(StakingError::InvalidAmount);
            }
            let stored_admin = get_admin(&env)?;
            if admin != stored_admin {
                return Err(StakingError::Unauthorized);
            }
            admin.require_auth();

            let reward_token = get_reward_token(&env)?;
            token::Client::new(&env, &reward_token).transfer(
                &admin,
                &env.current_contract_address(),
                &amount,
            );

            let total_staked = get_total_staked_internal(&env)?;
            if total_staked > 0 {
                // Fold any previously held rewards in first, then the new amount.
                let held = undistributed_rewards(&env);
                let rpt = reward_per_token(&env);
                let new_rpt = rpt + (held + amount) * REWARD_SCALE / total_staked;
                env.storage()
                    .instance()
                    .set(&DataKey::RewardPerTokenStored, &new_rpt);
                env.storage()
                    .instance()
                    .set(&DataKey::UndistributedRewards, &0i128);
            } else {
                // No stakers yet — hold the rewards until someone stakes.
                let held = undistributed_rewards(&env);
                env.storage()
                    .instance()
                    .set(&DataKey::UndistributedRewards, &(held + amount));
            }

            let total_rewards = get_total_rewards_internal(&env)?;
            let new_total = total_rewards + amount;
            env.storage()
                .instance()
                .set(&DataKey::TotalRewards, &new_total);

            bump(&env);
            events::rewards_added(&env, &admin, amount);
            Ok(())
        }

        /// Reclaim rewards that were deposited while no tokens were staked.
        ///
        /// Admin-only escape hatch for the case where the pool never receives a
        /// staker and the held rewards would otherwise be stuck (#1129).
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::Unauthorized`] if `admin` is not the stored admin.
        /// - [`StakingError::InvalidAmount`] if there are no undistributed rewards.
        pub fn reclaim_undistributed_rewards(
            env: Env,
            admin: Address,
        ) -> Result<i128, StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            let stored_admin = get_admin(&env)?;
            if admin != stored_admin {
                return Err(StakingError::Unauthorized);
            }
            admin.require_auth();

            let held = undistributed_rewards(&env);
            if held <= 0 {
                return Err(StakingError::InvalidAmount);
            }

            let reward_token = get_reward_token(&env)?;
            token::Client::new(&env, &reward_token).transfer(
                &env.current_contract_address(),
                &admin,
                &held,
            );

            env.storage()
                .instance()
                .set(&DataKey::UndistributedRewards, &0i128);

            let total_rewards = get_total_rewards_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalRewards, &(total_rewards - held));

            bump(&env);
            Ok(held)
        }

        /// Returns the amount of rewards currently held for future stakers.
        pub fn get_undistributed_rewards(env: Env) -> i128 {
            undistributed_rewards(&env)
        }

        /// Returns the total amount of stake tokens currently staked.
        pub fn get_total_staked(env: Env) -> Result<i128, StakingError> {
            get_total_staked_internal(&env)
        }

        /// Returns the total amount of reward tokens deposited into the pool.
        pub fn get_total_rewards(env: Env) -> Result<i128, StakingError> {
            get_total_rewards_internal(&env)
        }

        /// Returns the rewards currently claimable by `staker`.
        pub fn get_earned(env: Env, staker: Address) -> i128 {
            earned(&env, &staker)
        }

        /// Returns the current global reward-per-token accumulator.
        pub fn get_reward_per_token(env: Env) -> i128 {
            reward_per_token(&env)
        }
    }
}
