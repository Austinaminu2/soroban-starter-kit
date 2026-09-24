#![no_std]
#![deny(missing_docs)]
//! Atomic and basket token swaps with optional escrow and partial fills.

#[cfg(test)]
extern crate std;

use soroban_sdk::{Address, Env, Vec, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

pub use errors::SwapError;
pub use storage::{BasketLeg, BasketSwapInfo, DataKey, SwapInfo, SwapKey, SwapState};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, apply_bps_fee};
use storage::DataKey::{Admin, BasketSwapCount, FeeBps, SwapCount, Treasury};

fn extend_ttl_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn extend_ttl_persistent<K>(env: &Env, key: &K)
where
    K: soroban_sdk::TryIntoVal<Env, soroban_sdk::Val> + soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

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

fn get_required<V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &impl soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
) -> Result<V, SwapError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(SwapError::NotInitialized)
}

fn ensure_valid_leg_amount(amount: i128) -> Result<(), SwapError> {
    if amount <= 0 {
        return Err(SwapError::InvalidAmount);
    }
    Ok(())
}

fn map_state_err(state: SwapState) -> SwapError {
    match state {
        SwapState::Executed => SwapError::AlreadyCompleted,
        SwapState::Cancelled => SwapError::AlreadyCancelled,
        SwapState::Pending => SwapError::InvalidState,
    }
}

fn require_pending(state: SwapState) -> Result<(), SwapError> {
    if state != SwapState::Pending {
        return Err(map_state_err(state));
    }
    Ok(())
}

/// Atomic swap contract.
pub use contract::*;

mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct SwapContract;

    #[contractimpl]
    impl SwapContract {
        pub fn initialize(env: Env, admin: Address, treasury: Address, fee_bps: u32) -> Result<(), SwapError> {
            if env.storage().instance().has(&DataKey::Initialized) {
                return Err(SwapError::AlreadyInitialized);
            }
            if fee_bps > 10_000 {
                return Err(SwapError::InvalidFee);
            }
            admin.require_auth();

            env.storage().instance().set(&DataKey::Initialized, &true);
            env.storage().instance().set(&Admin, &admin);
            env.storage().instance().set(&Treasury, &treasury);
            env.storage().instance().set(&FeeBps, &fee_bps);
            env.storage().instance().set(&SwapCount, &0u32);
            env.storage().instance().set(&BasketSwapCount, &0u32);

            extend_ttl_instance(&env);
            bump_instance(&env);
            events::initialized(&env, &admin, fee_bps);
            Ok(())
        }

        pub fn set_treasury(env: Env, new_treasury: Address) -> Result<(), SwapError> {
            let admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();
            env.storage().instance().set(&Treasury, &new_treasury);
            bump_instance(&env);
            Ok(())
        }

        pub fn set_fee_bps(env: Env, new_fee_bps: u32) -> Result<(), SwapError> {
            let admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();
            if new_fee_bps > 10_000 {
                return Err(SwapError::InvalidFee);
            }
            env.storage().instance().set(&FeeBps, &new_fee_bps);
            bump_instance(&env);
            events::fee_updated(&env, &admin, new_fee_bps);
            Ok(())
        }

        pub fn set_admin(env: Env, new_admin: Address) -> Result<(), SwapError> {
            let admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();
            env.storage().instance().set(&Admin, &new_admin);
            bump_instance(&env);
            Ok(())
        }

        pub fn get_admin(env: Env) -> Result<Address, SwapError> {
            get_required(&env, &Admin)
        }

        pub fn get_treasury(env: Env) -> Result<Address, SwapError> {
            get_required(&env, &Treasury)
        }

        pub fn get_fee_bps(env: Env) -> Result<u32, SwapError> {
            get_required(&env, &FeeBps)
        }

        pub fn swap_count(env: Env) -> u32 {
            env.storage().instance().get(&SwapCount).unwrap_or(0)
        }

        pub fn basket_swap_count(env: Env) -> u32 {
            env.storage().instance().get(&BasketSwapCount).unwrap_or(0)
        }

        pub fn propose_swap(
            env: Env,
            party_a: Address,
            token_a: Address,
            amount_a: i128,
            token_b: Address,
            amount_b: i128,
            expires_at: u32,
        ) -> Result<u32, SwapError> {
            Self::propose_swap_with_options(
                env, party_a, token_a, amount_a, token_b, amount_b, expires_at, false, false,
            )
        }

        #[allow(clippy::too_many_arguments)]
        pub fn propose_swap_with_options(
            env: Env,
            party_a: Address,
            token_a: Address,
            amount_a: i128,
            token_b: Address,
            amount_b: i128,
            expires_at: u32,
            allow_partial: bool,
            escrowed: bool,
        ) -> Result<u32, SwapError> {
            get_required::<bool>(&env, &DataKey::Initialized)?;
            party_a.require_auth();
            ensure_valid_leg_amount(amount_a)?;
            ensure_valid_leg_amount(amount_b)?;
            if expires_at <= env.ledger().sequence() {
                return Err(SwapError::InvalidDeadline);
            }

            let swap_id: u32 = env.storage().instance().get(&SwapCount).unwrap_or(0);
            let next_id = swap_id.checked_add(1).ok_or(SwapError::StorageError)?;

            if escrowed {
                token::Client::new(&env, &token_a).transfer(
                    &party_a,
                    &env.current_contract_address(),
                    &amount_a,
                );
            }

            let swap = SwapInfo {
                id: swap_id,
                party_a: party_a.clone(),
                token_a: token_a.clone(),
                amount_a,
                token_b: token_b.clone(),
                amount_b,
                expires_at,
                state: SwapState::Pending,
                filled_amount: 0,
                allow_partial,
                escrowed,
            };

            env.storage().persistent().set(&SwapKey::Swap(swap_id), &swap);
            env.storage().instance().set(&SwapCount, &next_id);
            extend_ttl_persistent(&env, &SwapKey::Swap(swap_id));
            bump_persistent(&env, &SwapKey::Swap(swap_id));
            bump_instance(&env);
            events::swap_proposed(
                &env, &party_a, swap_id, &token_a, amount_a, &token_b, amount_b, expires_at,
            );
            Ok(swap_id)
        }

        pub fn accept_swap(env: Env, swap_id: u32, party_b: Address) -> Result<u32, SwapError> {
            let swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&SwapKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            require_pending(swap.state)?;
            if env.ledger().sequence() > swap.expires_at {
                return Err(SwapError::DeadlineExpired);
            }

            let remaining = swap
                .amount_a
                .checked_sub(swap.filled_amount)
                .ok_or(SwapError::MathOverflow)?;
            Self::accept_swap_partial(env, party_b, swap_id, remaining)
        }

        pub fn accept_swap_partial(
            env: Env,
            taker: Address,
            swap_id: u32,
            fill_amount_a: i128,
        ) -> Result<u32, SwapError> {
            taker.require_auth();
            ensure_valid_leg_amount(fill_amount_a)?;

            let fee_bps: u32 = get_required(&env, &FeeBps)?;
            let treasury: Address = get_required(&env, &Treasury)?;
            let mut swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&SwapKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            require_pending(swap.state)?;
            if env.ledger().sequence() > swap.expires_at {
                return Err(SwapError::DeadlineExpired);
            }

            let remaining_a = swap
                .amount_a
                .checked_sub(swap.filled_amount)
                .ok_or(SwapError::MathOverflow)?;
            if fill_amount_a > remaining_a {
                return Err(SwapError::InvalidAmount);
            }
            if !swap.allow_partial && fill_amount_a != remaining_a {
                return Err(SwapError::InvalidState);
            }

            let product = swap
                .amount_b
                .checked_mul(fill_amount_a)
                .ok_or(SwapError::MathOverflow)?;
            if product % swap.amount_a != 0 {
                return Err(SwapError::InvalidAmount);
            }
            let fill_amount_b = product / swap.amount_a;
            ensure_valid_leg_amount(fill_amount_b)?;

            let fee = apply_bps_fee(fill_amount_b, fee_bps).ok_or(SwapError::MathOverflow)?;
            let party_a_amount = fill_amount_b
                .checked_sub(fee)
                .ok_or(SwapError::MathOverflow)?;

            token::Client::new(&env, &swap.token_b).transfer(
                &taker,
                &env.current_contract_address(),
                &fill_amount_b,
            );
            token::Client::new(&env, &swap.token_b).transfer(
                &env.current_contract_address(),
                &swap.party_a,
                &party_a_amount,
            );
            if fee > 0 {
                token::Client::new(&env, &swap.token_b).transfer(
                    &env.current_contract_address(),
                    &treasury,
                    &fee,
                );
            }

            if swap.escrowed {
                token::Client::new(&env, &swap.token_a).transfer(
                    &env.current_contract_address(),
                    &taker,
                    &fill_amount_a,
                );
            } else {
                token::Client::new(&env, &swap.token_a).transfer_from(
                    &env.current_contract_address(),
                    &swap.party_a,
                    &taker,
                    &fill_amount_a,
                );
            }

            swap.filled_amount = swap
                .filled_amount
                .checked_add(fill_amount_a)
                .ok_or(SwapError::MathOverflow)?;
            if swap.filled_amount == swap.amount_a {
                swap.state = SwapState::Executed;
            }

            env.storage().persistent().set(&SwapKey::Swap(swap_id), &swap);
            extend_ttl_persistent(&env, &SwapKey::Swap(swap_id));
            bump_persistent(&env, &SwapKey::Swap(swap_id));
            bump_instance(&env);
            events::swap_accepted(&env, &taker, swap_id, fill_amount_a);
            Ok(swap_id)
        }

        pub fn cancel_swap(env: Env, swap_id: u32) -> Result<(), SwapError> {
            let mut swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&SwapKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            require_pending(swap.state)?;

            let now = env.ledger().sequence();
            if now <= swap.expires_at {
                swap.party_a.require_auth();
            }

            if swap.escrowed {
                let remaining_a = swap
                    .amount_a
                    .checked_sub(swap.filled_amount)
                    .ok_or(SwapError::MathOverflow)?;
                if remaining_a > 0 {
                    token::Client::new(&env, &swap.token_a).transfer(
                        &env.current_contract_address(),
                        &swap.party_a,
                        &remaining_a,
                    );
                }
            }

            swap.state = SwapState::Cancelled;
            env.storage().persistent().set(&SwapKey::Swap(swap_id), &swap);
            extend_ttl_persistent(&env, &SwapKey::Swap(swap_id));
            bump_persistent(&env, &SwapKey::Swap(swap_id));
            bump_instance(&env);
            events::swap_cancelled(&env, &swap.party_a, swap_id);
            Ok(())
        }

        pub fn get_swap(env: Env, swap_id: u32) -> Result<SwapInfo, SwapError> {
            env.storage()
                .persistent()
                .get(&SwapKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)
        }

        pub fn propose_basket_swap(
            env: Env,
            party_a: Address,
            offers: Vec<BasketLeg>,
            demands: Vec<BasketLeg>,
            expires_at: u32,
        ) -> Result<u32, SwapError> {
            get_required::<bool>(&env, &DataKey::Initialized)?;
            party_a.require_auth();
            if expires_at <= env.ledger().sequence() {
                return Err(SwapError::InvalidDeadline);
            }
            if offers.is_empty() || demands.is_empty() {
                return Err(SwapError::InvalidAmount);
            }
            for leg in offers.iter() {
                ensure_valid_leg_amount(leg.amount)?;
            }
            for leg in demands.iter() {
                ensure_valid_leg_amount(leg.amount)?;
            }

            let swap_id: u32 = env.storage().instance().get(&BasketSwapCount).unwrap_or(0);
            let next_id = swap_id.checked_add(1).ok_or(SwapError::StorageError)?;
            let swap = BasketSwapInfo {
                id: swap_id,
                party_a: party_a.clone(),
                offers,
                demands,
                expires_at,
                state: SwapState::Pending,
            };
            env.storage()
                .persistent()
                .set(&SwapKey::BasketSwap(swap_id), &swap);
            env.storage().instance().set(&BasketSwapCount, &next_id);
            extend_ttl_persistent(&env, &SwapKey::BasketSwap(swap_id));
            bump_persistent(&env, &SwapKey::BasketSwap(swap_id));
            bump_instance(&env);
            events::basket_swap_proposed(&env, &party_a, swap_id, expires_at);
            Ok(swap_id)
        }

        pub fn accept_basket_swap(env: Env, swap_id: u32, party_b: Address) -> Result<u32, SwapError> {
            party_b.require_auth();
            let mut swap: BasketSwapInfo = env
                .storage()
                .persistent()
                .get(&SwapKey::BasketSwap(swap_id))
                .ok_or(SwapError::BasketSwapNotFound)?;
            require_pending(swap.state)?;
            if env.ledger().sequence() > swap.expires_at {
                return Err(SwapError::DeadlineExpired);
            }

            // Two legs execute in one invocation; host failure rolls the full call back.
            for leg in swap.demands.iter() {
                token::Client::new(&env, &leg.token).transfer(&party_b, &swap.party_a, &leg.amount);
            }
            for leg in swap.offers.iter() {
                token::Client::new(&env, &leg.token).transfer_from(
                    &env.current_contract_address(),
                    &swap.party_a,
                    &party_b,
                    &leg.amount,
                );
            }

            swap.state = SwapState::Executed;
            env.storage()
                .persistent()
                .set(&SwapKey::BasketSwap(swap_id), &swap);
            extend_ttl_persistent(&env, &SwapKey::BasketSwap(swap_id));
            bump_persistent(&env, &SwapKey::BasketSwap(swap_id));
            bump_instance(&env);
            events::basket_swap_accepted(&env, &party_b, swap_id);
            Ok(swap_id)
        }

        pub fn get_basket_swap(env: Env, swap_id: u32) -> Result<BasketSwapInfo, SwapError> {
            env.storage()
                .persistent()
                .get(&SwapKey::BasketSwap(swap_id))
                .ok_or(SwapError::BasketSwapNotFound)
        }
    }
}

mod test;

#[cfg(test)]
mod prop_test;
