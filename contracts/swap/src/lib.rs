#![no_std]
#![deny(missing_docs)]
//! Atomic two-party token swap contract template.
//!
//! Party A escrows tokens when proposing a swap. Party B supplies the matching
//! tokens on acceptance; cancellation returns the escrow to Party A.

use soroban_sdk::{Address, Env, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

pub use errors::SwapError;
pub use storage::{DataKey, SwapInfo, SwapState};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, apply_bps_fee};

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn bump_swap(env: &Env, id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::Swap(id),
        LEDGER_LIFETIME_THRESHOLD,
        LEDGER_BUMP_AMOUNT,
    );
}

fn get_instance<V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<V, SwapError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(SwapError::NotInitialized)
}

/// Atomic two-party token swap.
pub use contract::*;

mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct SwapContract;

    #[contractimpl]
    impl SwapContract {
        /// Initialize the swap contract.
        pub fn initialize(
            env: Env,
            admin: Address,
            treasury: Address,
            fee_bps: u32,
        ) -> Result<(), SwapError> {
            if env.storage().instance().has(&DataKey::Initialized) {
                return Err(SwapError::AlreadyInitialized);
            }
            if fee_bps > 10_000 {
                return Err(SwapError::InvalidFee);
            }
            admin.require_auth();
            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage().instance().set(&DataKey::Treasury, &treasury);
            env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
            env.storage().instance().set(&DataKey::SwapCount, &0u32);
            env.storage().instance().set(&DataKey::Initialized, &true);
            bump_instance(&env);
            Ok(())
        }

        /// Set the fee recipient. Only the administrator may call this.
        pub fn set_treasury(env: Env, new_treasury: Address) -> Result<(), SwapError> {
            let admin: Address = get_instance(&env, &DataKey::Admin)?;
            admin.require_auth();
            env.storage()
                .instance()
                .set(&DataKey::Treasury, &new_treasury);
            bump_instance(&env);
            Ok(())
        }

        /// Set the fee in basis points. Only the administrator may call this.
        pub fn set_fee_bps(env: Env, new_fee_bps: u32) -> Result<(), SwapError> {
            let admin: Address = get_instance(&env, &DataKey::Admin)?;
            admin.require_auth();
            if new_fee_bps > 10_000 {
                return Err(SwapError::InvalidFee);
            }
            env.storage().instance().set(&DataKey::FeeBps, &new_fee_bps);
            bump_instance(&env);
            Ok(())
        }

        /// Set the administrator address. Only the current administrator may call this.
        pub fn set_admin(env: Env, new_admin: Address) -> Result<(), SwapError> {
            let admin: Address = get_instance(&env, &DataKey::Admin)?;
            admin.require_auth();
            env.storage().instance().set(&DataKey::Admin, &new_admin);
            bump_instance(&env);
            Ok(())
        }

        /// Return the configured administrator.
        pub fn get_admin(env: Env) -> Result<Address, SwapError> {
            get_instance(&env, &DataKey::Admin)
        }

        /// Return the configured treasury.
        pub fn get_treasury(env: Env) -> Result<Address, SwapError> {
            get_instance(&env, &DataKey::Treasury)
        }

        /// Return the configured fee in basis points.
        pub fn get_fee_bps(env: Env) -> Result<u32, SwapError> {
            get_instance(&env, &DataKey::FeeBps)
        }

        /// Return the number of swaps created so far.
        pub fn swap_count(env: Env) -> Result<u32, SwapError> {
            get_instance(&env, &DataKey::SwapCount)
        }

        /// Propose a swap and escrow Party A's asset in persistent storage.
        pub fn propose_swap(
            env: Env,
            party_a: Address,
            token_a: Address,
            amount_a: i128,
            token_b: Address,
            amount_b: i128,
            expires_at: u32,
        ) -> Result<u32, SwapError> {
            let _admin: Address = get_instance(&env, &DataKey::Admin)?;
            party_a.require_auth();
            if amount_a <= 0 || amount_b <= 0 {
                return Err(SwapError::InvalidAmount);
            }
            if expires_at <= env.ledger().sequence() {
                return Err(SwapError::InvalidDeadline);
            }
            let id: u32 = get_instance(&env, &DataKey::SwapCount)?;
            let next_id = id.checked_add(1).ok_or(SwapError::InvalidAmount)?;

            token::Client::new(&env, &token_a).transfer(
                &party_a,
                &env.current_contract_address(),
                &amount_a,
            );
            let swap = SwapInfo {
                id,
                party_a: party_a.clone(),
                token_a: token_a.clone(),
                amount_a,
                token_b: token_b.clone(),
                amount_b,
                expires_at,
                state: SwapState::Open,
            };
            env.storage().persistent().set(&DataKey::Swap(id), &swap);
            env.storage().instance().set(&DataKey::SwapCount, &next_id);
            bump_swap(&env, id);
            bump_instance(&env);
            events::swap_proposed(
                &env, &party_a, id, &token_a, amount_a, &token_b, amount_b, expires_at,
            );
            Ok(id)
        }

        /// Accept an open swap and atomically exchange both parties' assets.
        pub fn accept_swap(env: Env, swap_id: u32, party_b: Address) -> Result<u32, SwapError> {
            let treasury: Address = get_instance(&env, &DataKey::Treasury)?;
            let fee_bps: u32 = get_instance(&env, &DataKey::FeeBps)?;
            party_b.require_auth();
            let mut swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            match swap.state {
                SwapState::Completed => return Err(SwapError::AlreadyCompleted),
                SwapState::Cancelled => return Err(SwapError::AlreadyCancelled),
                SwapState::Open => {}
            }
            if env.ledger().sequence() > swap.expires_at {
                return Err(SwapError::DeadlineExpired);
            }
            let fee = apply_bps_fee(swap.amount_b, fee_bps).unwrap_or(0);
            let party_a_amount = swap
                .amount_b
                .checked_sub(fee)
                .ok_or(SwapError::InvalidAmount)?;

            // Effects are recorded before interactions; Soroban transactions revert
            // all writes if a token transfer fails.
            swap.state = SwapState::Completed;
            env.storage()
                .persistent()
                .set(&DataKey::Swap(swap_id), &swap);
            bump_swap(&env, swap_id);
            token::Client::new(&env, &swap.token_b).transfer(
                &party_b,
                &env.current_contract_address(),
                &swap.amount_b,
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
            token::Client::new(&env, &swap.token_a).transfer(
                &env.current_contract_address(),
                &party_b,
                &swap.amount_a,
            );
            bump_instance(&env);
            events::swap_accepted(&env, &party_b, swap_id);
            Ok(swap_id)
        }

        /// Cancel an open swap and return Party A's escrowed asset.
        pub fn cancel_swap(env: Env, swap_id: u32) -> Result<(), SwapError> {
            let mut swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            match swap.state {
                SwapState::Completed => return Err(SwapError::AlreadyCompleted),
                SwapState::Cancelled => return Err(SwapError::AlreadyCancelled),
                SwapState::Open => {}
            }
            swap.party_a.require_auth();
            swap.state = SwapState::Cancelled;
            env.storage()
                .persistent()
                .set(&DataKey::Swap(swap_id), &swap);
            bump_swap(&env, swap_id);
            token::Client::new(&env, &swap.token_a).transfer(
                &env.current_contract_address(),
                &swap.party_a,
                &swap.amount_a,
            );
            bump_instance(&env);
            events::swap_cancelled(&env, swap_id);
            Ok(())
        }

        /// Return a swap from persistent storage.
        pub fn get_swap(env: Env, swap_id: u32) -> Result<SwapInfo, SwapError> {
            let swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            bump_swap(&env, swap_id);
            Ok(swap)
        }
    }
}

#[cfg(test)]
mod prop_test;
mod test;
