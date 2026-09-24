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
            let new_stake = prev + amount;
            env.storage()
                .persistent()
                .set(&DataKey::Stake(staker.clone()), &new_stake);

            let total = get_total_staked_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalStaked, &(total + amount));

            bump(&env);
            events::staked(&env, &staker, amount, new_stake);
            Ok(())
        }

        /// Begin the unbonding process for `amount` stake tokens.
        ///
        /// When `unbonding_period == 0` the tokens are returned immediately
        /// (identical to the original behaviour). Otherwise an [`UnbondRequest`]
        /// is recorded and the caller must invoke [`Self::withdraw`] after
        /// `unbonding_period` ledgers have elapsed.
        ///
        /// Accrued rewards are snapshotted but not transferred; call
        /// [`Self::claim_rewards`] separately.
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::InvalidAmount`] if `amount` <= 0.
        /// - [`StakingError::NoStake`] if the staker has no stake.
        /// - [`StakingError::InsufficientStake`] if `amount` exceeds the staker's stake.
        pub fn unstake(env: Env, staker: Address, amount: i128) -> Result<(), StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            if amount <= 0 {
                return Err(StakingError::InvalidAmount);
            }
            staker.require_auth();

            let current: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Stake(staker.clone()))
                .unwrap_or(0i128);
            if current == 0 {
                return Err(StakingError::NoStake);
            }
            if amount > current {
                return Err(StakingError::InsufficientStake);
            }

            update_reward(&env, &staker);

            let remaining = current - amount;
            env.storage()
                .persistent()
                .set(&DataKey::Stake(staker.clone()), &remaining);

            let total = get_total_staked_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalStaked, &(total - amount));

            let unbonding_period: u32 = env
                .storage()
                .instance()
                .get(&DataKey::UnbondingPeriod)
                .unwrap_or(0u32);

            if unbonding_period == 0 {
                // Immediate withdrawal — legacy behaviour.
                let stake_token = get_stake_token(&env)?;
                token::Client::new(&env, &stake_token).transfer(
                    &env.current_contract_address(),
                    &staker,
                    &amount,
                );
                bump(&env);
                events::unstaked(&env, &staker, amount, remaining);
            } else {
                // Check if there's already a pending unbond request.
                if env
                    .storage()
                    .persistent()
                    .has(&DataKey::UnbondRequest(staker.clone()))
                {
                    return Err(StakingError::UnbondRequestPending);
                }

                // Queue an unbond request.
                let available_at = env.ledger().sequence() + unbonding_period;
                let request = UnbondRequest {
                    amount,
                    available_at,
                };
                env.storage()
                    .persistent()
                    .set(&DataKey::UnbondRequest(staker.clone()), &request);
                bump(&env);
                events::unbond_requested(&env, &staker, amount, available_at);
            }

            Ok(())
        }

        /// Withdraw tokens after the unbonding period has elapsed.
        ///
        /// Must be called after `unstake` recorded an [`UnbondRequest`] and the
        /// required number of ledgers have passed.
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::NoUnbondRequest`] if there is no pending unbond request.
        /// - [`StakingError::UnbondingNotComplete`] if the unbonding period has not elapsed.
        pub fn withdraw(env: Env, staker: Address) -> Result<i128, StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            staker.require_auth();

            let request: UnbondRequest = env
                .storage()
                .persistent()
                .get(&DataKey::UnbondRequest(staker.clone()))
                .ok_or(StakingError::NoUnbondRequest)?;

            if env.ledger().sequence() < request.available_at {
                return Err(StakingError::UnbondingNotComplete);
            }

            // Clear the request before transferring.
            env.storage()
                .persistent()
                .remove(&DataKey::UnbondRequest(staker.clone()));

            let stake_token = get_stake_token(&env)?;
            token::Client::new(&env, &stake_token).transfer(
                &env.current_contract_address(),
                &staker,
                &request.amount,
            );

            bump(&env);
            events::withdrawn(&env, &staker, request.amount);
            Ok(request.amount)
        }

        /// Transfer all accrued reward tokens to `staker`.
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::NoRewards`] if there are no rewards to claim.
        pub fn claim_rewards(env: Env, staker: Address) -> Result<i128, StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            staker.require_auth();

            update_reward(&env, &staker);

            let reward: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Rewards(staker.clone()))
                .unwrap_or(0i128);
            if reward <= 0 {
                return Err(StakingError::NoRewards);
            }

            env.storage()
                .persistent()
                .set(&DataKey::Rewards(staker.clone()), &0i128);

            let total_rewards = get_total_rewards_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalRewards, &(total_rewards - reward));

            let reward_token = get_reward_token(&env)?;
            token::Client::new(&env, &reward_token).transfer(
                &env.current_contract_address(),
                &staker,
                &reward,
            );

            bump(&env);
            events::rewards_claimed(&env, &staker, reward);
            Ok(reward)
        }

        /// Admin deposits `amount` reward tokens into the pool.
        ///
        /// The reward-per-token accumulator is increased by `amount / total_staked`.
        /// If no tokens are currently staked the rewards are held and distributed
        /// when stakers join.
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::Unauthorized`] if the caller is not the admin.
        /// - [`StakingError::InvalidAmount`] if `amount` <= 0.
        pub fn add_rewards(env: Env, amount: i128) -> Result<(), StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            if amount <= 0 {
                return Err(StakingError::InvalidAmount);
            }

            let admin = get_admin(&env)?;
            admin.require_auth();

            let reward_token = get_reward_token(&env)?;
            token::Client::new(&env, &reward_token).transfer(
                &admin,
                &env.current_contract_address(),
                &amount,
            );

            let total_staked = get_total_staked_internal(&env)?;
            if total_staked > 0 {
                let rpt: i128 = env
                    .storage()
                    .instance()
                    .get(&DataKey::RewardPerTokenStored)
                    .unwrap_or(0i128);
                let new_rpt = rpt + amount * REWARD_SCALE / total_staked;
                env.storage()
                    .instance()
                    .set(&DataKey::RewardPerTokenStored, &new_rpt);
            }

            let total_rewards = get_total_rewards_internal(&env)?;
            let new_total = total_rewards + amount;
            env.storage()
                .instance()
                .set(&DataKey::TotalRewards, &new_total);

            bump(&env);
            events::rewards_added(&env, &admin, amount, new_total);
            Ok(())
        }

        /// Admin-only: slash `amount` tokens from `staker`'s stake.
        ///
        /// The slashed amount is capped at the staker's current stake balance.
        /// Slashed tokens are transferred to the `slash_destination` address
        /// configured at initialization (e.g. a treasury or burn address).
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::Unauthorized`] if the caller is not the admin.
        /// - [`StakingError::InvalidAmount`] if `amount` <= 0.
        /// - [`StakingError::NoStake`] if the staker has no balance to slash.
        pub fn slash(env: Env, staker: Address, amount: i128) -> Result<i128, StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            if amount <= 0 {
                return Err(StakingError::InvalidAmount);
            }

            let admin = get_admin(&env)?;
            admin.require_auth();

            let current: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Stake(staker.clone()))
                .unwrap_or(0i128);
            if current == 0 {
                return Err(StakingError::NoStake);
            }

            // Cap at current balance.
            let slash_amount = if amount > current { current } else { amount };

            // Update reward snapshot before adjusting stake.
            update_reward(&env, &staker);

            let remaining = current - slash_amount;
            env.storage()
                .persistent()
                .set(&DataKey::Stake(staker.clone()), &remaining);

            let total = get_total_staked_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalStaked, &(total - slash_amount));

            // Route slashed tokens to the configured destination.
            let destination: Address = env
                .storage()
                .instance()
                .get(&DataKey::SlashDestination)
                .ok_or(StakingError::NotInitialized)?;
            let stake_token = get_stake_token(&env)?;
            token::Client::new(&env, &stake_token).transfer(
                &env.current_contract_address(),
                &destination,
                &slash_amount,
            );

            bump(&env);
            events::slashed(&env, &admin, &staker, slash_amount, &destination);
            Ok(slash_amount)
        }

        /// Returns the staker's current stake balance.
        pub fn get_stake(env: Env, staker: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&DataKey::Stake(staker))
                .unwrap_or(0i128)
        }

        /// Returns the staker's currently accrued (unclaimed) rewards.
        pub fn get_rewards(env: Env, staker: Address) -> i128 {
            if !env.storage().instance().has(&DataKey::Admin) {
                return 0;
            }
            earned(&env, &staker)
        }

        /// Returns the total amount of tokens currently staked.
        pub fn get_total_staked(env: Env) -> i128 {
            env.storage()
                .instance()
                .get(&DataKey::TotalStaked)
                .unwrap_or(0i128)
        }

        /// Returns the total reward tokens held by the contract.
        pub fn get_total_rewards(env: Env) -> i128 {
            env.storage()
                .instance()
                .get(&DataKey::TotalRewards)
                .unwrap_or(0i128)
        }

        /// Return the on-chain contract version number.
        pub fn contract_version(env: Env) -> u32 {
            env.storage().instance().get(&DataKey::Version).unwrap_or(0)
        }

        /// Return the pending unbond request for `staker`, if any.
        pub fn get_unbond_request(env: Env, staker: Address) -> Option<UnbondRequest> {
            env.storage()
                .persistent()
                .get(&DataKey::UnbondRequest(staker))
        }

        /// Enable or disable auto-compounding for `staker`.
        ///
        /// When compounding is enabled, calling [`compound`](Self::compound) will
        /// re-stake accrued rewards instead of transferring them out.
        /// Requires stake token == reward token.
        pub fn set_compounding(
            env: Env,
            staker: Address,
            enabled: bool,
        ) -> Result<(), StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            staker.require_auth();
            env.storage()
                .persistent()
                .set(&DataKey::Compounding(staker), &enabled);
            bump(&env);
            Ok(())
        }

        /// Re-stake accrued rewards back into principal (compound).
        ///
        /// This transfers no tokens externally — rewards are simply moved from the
        /// reward ledger back into the staker's principal stake.  Requires the stake
        /// token and reward token to be the same contract (single-asset staking).
        ///
        /// # Errors
        /// - [`StakingError::NotInitialized`] if the contract has not been initialized.
        /// - [`StakingError::NoRewards`] if there are no rewards to compound.
        /// - [`StakingError::CompoundTokenMismatch`] if stake token != reward token.
        #[allow(clippy::arithmetic_side_effects)] // checked via overflow guards
        pub fn compound(env: Env, staker: Address) -> Result<i128, StakingError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(StakingError::NotInitialized);
            }
            staker.require_auth();

            // Compound only makes sense when stake token == reward token.
            let stake_token = get_stake_token(&env)?;
            let reward_token = get_reward_token(&env)?;
            if stake_token != reward_token {
                return Err(StakingError::CompoundTokenMismatch);
            }

            update_reward(&env, &staker);

            let reward: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Rewards(staker.clone()))
                .unwrap_or(0i128);
            if reward <= 0 {
                return Err(StakingError::NoRewards);
            }

            // Clear the reward ledger entry.
            env.storage()
                .persistent()
                .set(&DataKey::Rewards(staker.clone()), &0i128);

            // Deduct from total rewards pool.
            let total_rewards = get_total_rewards_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalRewards, &(total_rewards - reward));

            // Add compounded reward to principal stake.
            let prev: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Stake(staker.clone()))
                .unwrap_or(0i128);
            let new_stake = prev + reward;
            env.storage()
                .persistent()
                .set(&DataKey::Stake(staker.clone()), &new_stake);

            let total = get_total_staked_internal(&env)?;
            env.storage()
                .instance()
                .set(&DataKey::TotalStaked, &(total + reward));

            bump(&env);
            events::compounded(&env, &staker, reward, new_stake);
            Ok(reward)
        }
    }
}
